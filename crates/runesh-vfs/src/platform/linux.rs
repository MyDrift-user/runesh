//! Linux FUSE implementation for the virtual filesystem.
//!
//! Uses the `fuser` crate to mount a FUSE filesystem that serves files
//! from a FileProvider with on-demand hydration and overlay write support.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    Request,
};
use tokio::sync::RwLock;

use crate::cache::CacheManager;
use crate::config::{VfsConfig, WriteMode};
use crate::error::VfsError;
use crate::provider::{FileProvider, VfsEntry};

const TTL: Duration = Duration::from_secs(5);
const ROOT_INO: u64 = 1;

/// Linux FUSE mount handle.
pub struct LinuxFuseMount {
    mount_point: PathBuf,
    session: fuser::BackgroundSession,
}

impl LinuxFuseMount {
    pub async fn mount(
        config: VfsConfig,
        provider: Arc<dyn FileProvider>,
        cache: Arc<CacheManager>,
    ) -> Result<Self, VfsError> {
        let mount_point = config.mount_point.clone();
        tokio::fs::create_dir_all(&mount_point).await?;

        let runtime = tokio::runtime::Handle::current();
        let write_mode = config.write_mode.clone();

        let fs = VfsFuse::new(provider, cache, write_mode, runtime);

        let mut options = vec![
            MountOption::FSName(config.display_name.clone()),
            MountOption::AutoUnmount,
            MountOption::AllowOther,
        ];

        if matches!(config.write_mode, WriteMode::ReadOnly) {
            options.push(MountOption::RO);
        }

        let mount_point_clone = mount_point.clone();
        let session = tokio::task::spawn_blocking(move || {
            fuser::spawn_mount2(fs, &mount_point_clone, &options)
                .map_err(|e| VfsError::Platform(format!("FUSE mount failed: {e}")))
        })
        .await
        .map_err(|e| VfsError::Platform(format!("Task join error: {e}")))?
        ?;

        tracing::info!(mount_point = %mount_point.display(), "FUSE: mounted");

        Ok(Self {
            mount_point,
            session,
        })
    }
}

impl super::VfsMountInner for LinuxFuseMount {
    fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}

impl Drop for LinuxFuseMount {
    fn drop(&mut self) {
        tracing::info!(mount_point = %self.mount_point.display(), "FUSE: unmounting");
        // BackgroundSession unmounts on drop
    }
}

/// FUSE filesystem implementation backed by a FileProvider.
struct VfsFuse {
    provider: Arc<dyn FileProvider>,
    cache: Arc<CacheManager>,
    write_mode: WriteMode,
    runtime: tokio::runtime::Handle,
    /// inode → path mapping
    inodes: RwLock<HashMap<u64, String>>,
    /// path → inode reverse mapping
    paths: RwLock<HashMap<String, u64>>,
    /// Next inode number to assign
    next_ino: AtomicU64,
}

impl VfsFuse {
    fn new(
        provider: Arc<dyn FileProvider>,
        cache: Arc<CacheManager>,
        write_mode: WriteMode,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let inodes = RwLock::new(HashMap::from([(ROOT_INO, String::new())]));
        let paths = RwLock::new(HashMap::from([(String::new(), ROOT_INO)]));

        Self {
            provider,
            cache,
            write_mode,
            runtime,
            inodes,
            paths,
            next_ino: AtomicU64::new(2),
        }
    }

    /// Get or assign an inode for a path.
    fn get_or_create_ino(&self, path: &str) -> u64 {
        // Try read lock first (fast path)
        if let Some(ino) = self.runtime.block_on(self.paths.read()).get(path) {
            return *ino;
        }

        // Assign new inode
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        self.runtime.block_on(async {
            self.inodes.write().await.insert(ino, path.to_string());
            self.paths.write().await.insert(path.to_string(), ino);
        });
        ino
    }

    /// Resolve an inode to a path.
    fn ino_to_path(&self, ino: u64) -> Option<String> {
        self.runtime
            .block_on(self.inodes.read())
            .get(&ino)
            .cloned()
    }

    /// Convert a VfsEntry to FUSE FileAttr.
    fn entry_to_attr(&self, entry: &VfsEntry, ino: u64) -> FileAttr {
        let kind = if entry.is_dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        };

        let mtime = entry
            .modified
            .unwrap_or(UNIX_EPOCH);
        let ctime = entry.created.unwrap_or(mtime);
        let atime = entry.accessed.unwrap_or(mtime);

        let perm = if entry.readonly { 0o555 } else { 0o755 };

        FileAttr {
            ino,
            size: entry.size,
            blocks: (entry.size + 511) / 512,
            atime,
            mtime,
            ctime,
            crtime: ctime,
            kind,
            perm,
            nlink: if entry.is_dir { 2 } else { 1 },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

impl Filesystem for VfsFuse {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let name_str = name.to_string_lossy();
        let child_path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        match self.runtime.block_on(self.provider.stat(&child_path)) {
            Ok(entry) => {
                let ino = self.get_or_create_ino(&child_path);
                let attr = self.entry_to_attr(&entry, ino);
                reply.entry(&TTL, &attr, 0);
            }
            Err(_) => {
                reply.error(libc::ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Root directory
        if ino == ROOT_INO {
            let attr = FileAttr {
                ino: ROOT_INO,
                size: 0,
                blocks: 0,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind: FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                blksize: 4096,
                flags: 0,
            };
            reply.attr(&TTL, &attr);
            return;
        }

        match self.runtime.block_on(self.provider.stat(&path)) {
            Ok(entry) => {
                let attr = self.entry_to_attr(&entry, ino);
                reply.attr(&TTL, &attr);
            }
            Err(_) => {
                reply.error(libc::ENOENT);
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let entries = match self.runtime.block_on(self.provider.list_dir(&path)) {
            Ok(e) => e,
            Err(_) => {
                reply.error(libc::EIO);
                return;
            }
        };

        // Standard . and .. entries
        let mut all_entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".into()),
            (ino, FileType::Directory, "..".into()),
        ];

        for entry in entries {
            let child_ino = self.get_or_create_ino(&entry.path);
            let kind = if entry.is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            all_entries.push((child_ino, kind, entry.name));
        }

        for (i, (ino, kind, name)) in all_entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*ino, (i + 1) as i64, *kind, name) {
                break; // Buffer full
            }
        }

        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Try cache first
        if let Some(cached) = self.runtime.block_on(self.cache.get(&path)) {
            let start = offset as usize;
            let end = (start + size as usize).min(cached.len());
            if start < cached.len() {
                reply.data(&cached[start..end]);
            } else {
                reply.data(&[]);
            }
            return;
        }

        // Fetch from provider and cache
        match self
            .runtime
            .block_on(self.provider.read_file(&path, offset as u64, size as u64))
        {
            Ok(data) => {
                // Cache the full file if this is a first read
                if offset == 0 {
                    let _ = self.runtime.block_on(self.cache.put(&path, &data));
                }
                reply.data(&data);
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(libc::EACCES);
            return;
        }

        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        match self
            .runtime
            .block_on(self.provider.write_file(&path, data, offset as u64))
        {
            Ok(()) => {
                // Invalidate cache
                let _ = self.runtime.block_on(self.cache.evict(&path));
                reply.written(data.len() as u32);
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(libc::EACCES);
            return;
        }

        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let name_str = name.to_string_lossy();
        let child_path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        match self.runtime.block_on(self.provider.mkdir(&child_path)) {
            Ok(()) => {
                let ino = self.get_or_create_ino(&child_path);
                let entry = VfsEntry::directory(&name_str, &child_path);
                let attr = self.entry_to_attr(&entry, ino);
                reply.entry(&TTL, &attr, 0);
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(libc::EACCES);
            return;
        }

        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let name_str = name.to_string_lossy();
        let child_path = if parent_path.is_empty() {
            name_str.to_string()
        } else {
            format!("{}/{}", parent_path, name_str)
        };

        match self.runtime.block_on(self.provider.delete(&child_path)) {
            Ok(()) => {
                let _ = self.runtime.block_on(self.cache.evict(&child_path));
                reply.ok();
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }
}
