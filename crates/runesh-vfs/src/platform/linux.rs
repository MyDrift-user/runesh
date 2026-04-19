//! Linux FUSE implementation for the virtual filesystem.
//!
//! Uses the `fuser` crate to mount a FUSE filesystem that serves files
//! from a FileProvider with on-demand hydration and overlay write support.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation, INodeNo, LockOwner,
    MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request, WriteFlags,
};
use tokio::sync::RwLock;

use crate::cache::CacheManager;
use crate::config::{VfsConfig, WriteMode};
use crate::error::VfsError;
use crate::provider::{FileProvider, VfsEntry};

const TTL: Duration = Duration::from_secs(5);
const ROOT_INO: INodeNo = INodeNo::ROOT;

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

        let mut mount_options = vec![
            MountOption::FSName(config.display_name.clone()),
            MountOption::AutoUnmount,
            MountOption::CUSTOM("allow_other".into()),
        ];

        if matches!(config.write_mode, WriteMode::ReadOnly) {
            mount_options.push(MountOption::RO);
        }

        let fuse_config = Config {
            mount_options,
            ..Config::default()
        };

        let mount_point_clone = mount_point.clone();
        let session = tokio::task::spawn_blocking(move || {
            fuser::spawn_mount2(fs, &mount_point_clone, &fuse_config)
                .map_err(|e| VfsError::Platform(format!("FUSE mount failed: {e}")))
        })
        .await
        .map_err(|e| VfsError::Platform(format!("Task join error: {e}")))??;

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
    /// inode -> path mapping
    inodes: RwLock<HashMap<INodeNo, String>>,
    /// path -> inode reverse mapping
    paths: RwLock<HashMap<String, INodeNo>>,
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
    fn get_or_create_ino(&self, path: &str) -> INodeNo {
        // Try read lock first (fast path)
        if let Some(ino) = self.runtime.block_on(self.paths.read()).get(path) {
            return *ino;
        }

        // Assign new inode
        let raw = self.next_ino.fetch_add(1, Ordering::Relaxed);
        let ino = INodeNo(raw);
        self.runtime.block_on(async {
            self.inodes.write().await.insert(ino, path.to_string());
            self.paths.write().await.insert(path.to_string(), ino);
        });
        ino
    }

    /// Resolve an inode to a path.
    fn ino_to_path(&self, ino: INodeNo) -> Option<String> {
        self.runtime.block_on(self.inodes.read()).get(&ino).cloned()
    }

    /// Convert a VfsEntry to FUSE FileAttr.
    fn entry_to_attr(&self, entry: &VfsEntry, ino: INodeNo) -> FileAttr {
        let kind = if entry.is_dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        };

        let mtime = entry.modified.unwrap_or(UNIX_EPOCH);
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
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
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
                reply.entry(&TTL, &attr, Generation(0));
            }
            Err(_) => {
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
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
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
                return;
            }
        };

        let entries = match self.runtime.block_on(self.provider.list_dir(&path)) {
            Ok(e) => e,
            Err(_) => {
                reply.error(Errno::EIO);
                return;
            }
        };

        // Standard . and .. entries
        let mut all_entries: Vec<(INodeNo, FileType, String)> = vec![
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
            if reply.add(*ino, (i + 1) as u64, *kind, name) {
                break; // Buffer full
            }
        }

        reply.ok();
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
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
            .block_on(self.provider.read_file(&path, offset, size as u64))
        {
            Ok(data) => {
                // Cache the full file if this is a first read
                if offset == 0 {
                    let _ = self.runtime.block_on(self.cache.put(&path, &data));
                }
                reply.data(&data);
            }
            Err(_) => {
                reply.error(Errno::EIO);
            }
        }
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: fuser::ReplyWrite,
    ) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(Errno::EACCES);
            return;
        }

        let path = match self.ino_to_path(ino) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
                return;
            }
        };

        match self
            .runtime
            .block_on(self.provider.write_file(&path, data, offset))
        {
            Ok(()) => {
                // Invalidate cache
                let _ = self.runtime.block_on(self.cache.evict(&path));
                reply.written(data.len() as u32);
            }
            Err(_) => {
                reply.error(Errno::EIO);
            }
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(Errno::EACCES);
            return;
        }

        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
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
                reply.entry(&TTL, &attr, Generation(0));
            }
            Err(_) => {
                reply.error(Errno::EIO);
            }
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: fuser::ReplyEmpty) {
        if matches!(self.write_mode, WriteMode::ReadOnly) {
            reply.error(Errno::EACCES);
            return;
        }

        let parent_path = match self.ino_to_path(parent) {
            Some(p) => p,
            None => {
                reply.error(Errno::ENOENT);
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
                reply.error(Errno::EIO);
            }
        }
    }
}
