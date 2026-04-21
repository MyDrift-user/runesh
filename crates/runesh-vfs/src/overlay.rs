//! OverlayProvider — copy-on-write layer for user-specific edits.
//!
//! Wraps a base FileProvider and redirects writes to a local overlay directory.
//! Reads check the overlay first, then fall back to the base provider.
//! Original files are never modified — only edited files consume storage.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use crate::error::VfsError;
use crate::provider::{FileProvider, VfsEntry};

/// Validate a relative path and resolve it under a base directory.
/// Rejects traversal (`..'`, null bytes, absolute paths).
fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, VfsError> {
    if relative.contains('\0') {
        return Err(VfsError::PathTraversal);
    }

    let mut result = base.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(c) => result.push(c),
            Component::CurDir => {}
            // ParentDir, RootDir, Prefix all escape the sandbox
            _ => return Err(VfsError::PathTraversal),
        }
    }

    if !result.starts_with(base) {
        return Err(VfsError::PathTraversal);
    }

    Ok(result)
}

/// Copy-on-write overlay provider.
///
/// - **Lower layer**: read-only base files from the origin FileProvider
/// - **Upper layer**: local modifications stored on disk
///
/// When a file is written, it is first copied from lower to upper (if not already there),
/// then the write is applied to the upper copy. The original is never touched.
pub struct OverlayProvider {
    /// Base file provider (read-only for overlay users).
    lower: Arc<dyn FileProvider>,
    /// Local directory for overlay files.
    upper_path: PathBuf,
    /// Set of files that exist in the upper layer.
    modified: RwLock<HashSet<String>>,
    /// Set of files deleted in the overlay (hidden from listings).
    deleted: RwLock<HashSet<String>>,
    /// Per-path locks serializing `copy_up` so two concurrent writes to
    /// the same file can't both observe `has_upper == false` and both
    /// attempt to copy from the lower layer.
    copy_up_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl OverlayProvider {
    /// Create a new overlay provider.
    pub async fn new(lower: Arc<dyn FileProvider>, upper_path: PathBuf) -> Result<Self, VfsError> {
        tokio::fs::create_dir_all(&upper_path).await?;

        // Scan upper directory for existing overlay files
        let mut modified = HashSet::new();
        scan_upper_dir(&upper_path, &upper_path, &mut modified).await;

        // Load deleted file list if it exists
        let deleted = load_deleted_list(&upper_path).await;

        Ok(Self {
            lower,
            upper_path,
            modified: RwLock::new(modified),
            deleted: RwLock::new(deleted),
            copy_up_locks: Mutex::new(HashMap::new()),
        })
    }

    /// Get the upper layer path for a given relative path.
    /// Returns an error if the path would escape the upper directory.
    fn upper_file(&self, path: &str) -> Result<PathBuf, VfsError> {
        safe_join(&self.upper_path, path)
    }

    /// Check if a file exists in the upper layer.
    async fn has_upper(&self, path: &str) -> bool {
        self.modified.read().await.contains(path)
    }

    /// Check if a file was deleted in the overlay.
    async fn is_deleted(&self, path: &str) -> bool {
        self.deleted.read().await.contains(path)
    }

    /// Copy a file from lower to upper layer (copy-on-write).
    ///
    /// Uses a per-path async mutex so two concurrent writes to the same
    /// file can't both observe `has_upper == false` and race to `write()`.
    async fn copy_up(&self, path: &str) -> Result<(), VfsError> {
        // Fast path: already copied up.
        if self.has_upper(path).await {
            return Ok(());
        }

        let path_lock = {
            let mut locks = self.copy_up_locks.lock().await;
            locks
                .entry(path.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = path_lock.lock().await;

        // Re-check under the lock.
        if self.has_upper(path).await {
            return Ok(());
        }

        let content = self.lower.read_file(path, 0, 0).await?;
        let upper_file = self.upper_file(path)?;

        if let Some(parent) = upper_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&upper_file, &content).await?;
        self.modified.write().await.insert(path.to_string());

        tracing::debug!(path = %path, "Overlay: copied up from base");
        Ok(())
    }

    /// Persist the deleted files list.
    async fn save_deleted_list(&self) {
        let deleted = self.deleted.read().await;
        let list_path = self.upper_path.join(".overlay-deleted.json");
        let json = serde_json::to_string(&*deleted).unwrap_or_default();
        let _ = tokio::fs::write(&list_path, json).await;
    }

    /// Get list of files modified in the overlay (for sync).
    pub async fn modified_files(&self) -> Vec<String> {
        self.modified.read().await.iter().cloned().collect()
    }
}

#[async_trait]
impl FileProvider for OverlayProvider {
    async fn list_dir(&self, path: &str) -> Result<Vec<VfsEntry>, VfsError> {
        // Get base listing
        let mut entries = self.lower.list_dir(path).await?;

        // Remove deleted files
        let deleted = self.deleted.read().await;
        entries.retain(|e| !deleted.contains(&e.path));
        drop(deleted);

        // Override with upper layer entries (modified files)
        let upper_dir = self.upper_file(path)?;
        if upper_dir.is_dir() {
            let mut read_dir = tokio::fs::read_dir(&upper_dir).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip overlay metadata files
                if name.starts_with(".overlay-") {
                    continue;
                }

                let entry_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", path, name)
                };

                let metadata = entry.metadata().await?;

                // Find and replace matching entry, or add new
                let vfs_entry = VfsEntry {
                    name: name.clone(),
                    path: entry_path.clone(),
                    is_dir: metadata.is_dir(),
                    size: metadata.len(),
                    created: metadata.created().ok(),
                    modified: metadata.modified().ok(),
                    accessed: metadata.accessed().ok(),
                    readonly: false,
                    is_hydrated: true,
                    content_hash: None,
                    content_type: None,
                    uid: None,
                    gid: None,
                };

                if let Some(existing) = entries.iter_mut().find(|e| e.name == name) {
                    *existing = vfs_entry;
                } else {
                    entries.push(vfs_entry);
                }
            }
        }

        // Sort: directories first, then alphabetically
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(entries)
    }

    async fn stat(&self, path: &str) -> Result<VfsEntry, VfsError> {
        if self.is_deleted(path).await {
            return Err(VfsError::NotFound(path.into()));
        }

        // Check upper first
        let upper_file = self.upper_file(path)?;
        if upper_file.exists() {
            let metadata = tokio::fs::metadata(&upper_file).await?;
            let name = upper_file
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            return Ok(VfsEntry {
                name,
                path: path.to_string(),
                is_dir: metadata.is_dir(),
                size: metadata.len(),
                created: metadata.created().ok(),
                modified: metadata.modified().ok(),
                accessed: metadata.accessed().ok(),
                readonly: false,
                is_hydrated: true,
                content_hash: None,
                content_type: None,
                uid: None,
                gid: None,
            });
        }

        // Fall back to base
        self.lower.stat(path).await
    }

    async fn read_file(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>, VfsError> {
        if self.is_deleted(path).await {
            return Err(VfsError::NotFound(path.into()));
        }

        // Check upper layer first
        let upper_file = self.upper_file(path)?;
        if upper_file.exists() {
            return read_from_disk(&upper_file, offset, length).await;
        }

        // Fall back to base provider
        self.lower.read_file(path, offset, length).await
    }

    async fn write_file(&self, path: &str, data: &[u8], offset: u64) -> Result<(), VfsError> {
        // Copy-on-write: ensure file exists in upper layer
        if !self.has_upper(path).await {
            // Try to copy from lower; if not found, create new
            let upper_file = self.upper_file(path)?;
            match self.lower.read_file(path, 0, 0).await {
                Ok(content) => {
                    if let Some(parent) = upper_file.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&upper_file, &content).await?;
                }
                Err(VfsError::NotFound(_)) => {
                    // New file — just create in upper
                    if let Some(parent) = upper_file.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&upper_file, &[]).await?;
                }
                Err(e) => return Err(e),
            }
            self.modified.write().await.insert(path.to_string());
        }

        // Write to upper layer
        let upper_file = self.upper_file(path)?;
        if offset == 0
            && data.len() as u64
                >= tokio::fs::metadata(&upper_file)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
        {
            // Full overwrite
            tokio::fs::write(&upper_file, data).await?;
        } else {
            // Partial write at offset
            use tokio::io::{AsyncSeekExt, AsyncWriteExt};
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&upper_file)
                .await?;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            file.write_all(data).await?;
        }

        // Un-delete if it was previously deleted
        self.deleted.write().await.remove(path);

        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), VfsError> {
        // Remove from upper layer if present
        let upper_file = self.upper_file(path)?;
        if upper_file.exists() {
            if upper_file.is_dir() {
                tokio::fs::remove_dir_all(&upper_file).await?;
            } else {
                tokio::fs::remove_file(&upper_file).await?;
            }
            self.modified.write().await.remove(path);
        }

        // Mark as deleted (hides from base layer too)
        self.deleted.write().await.insert(path.to_string());
        self.save_deleted_list().await;

        Ok(())
    }

    async fn mkdir(&self, path: &str) -> Result<(), VfsError> {
        let upper_dir = self.upper_file(path)?;
        tokio::fs::create_dir_all(&upper_dir).await?;
        self.modified.write().await.insert(path.to_string());
        self.deleted.write().await.remove(path);
        Ok(())
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        // Ensure source exists in upper (copy up if needed)
        self.copy_up(old_path).await?;

        let old_upper = self.upper_file(old_path)?;
        let new_upper = self.upper_file(new_path)?;

        if let Some(parent) = new_upper.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::rename(&old_upper, &new_upper).await?;

        let mut modified = self.modified.write().await;
        modified.remove(old_path);
        modified.insert(new_path.to_string());

        // Mark old path as deleted from base
        self.deleted.write().await.insert(old_path.to_string());
        self.save_deleted_list().await;

        Ok(())
    }
}

/// Read a file from disk with optional byte range.
async fn read_from_disk(path: &Path, offset: u64, length: u64) -> Result<Vec<u8>, VfsError> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;

    if offset > 0 {
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    if length > 0 {
        let mut buf = vec![0u8; length as usize];
        let n = file.read(&mut buf).await?;
        buf.truncate(n);
        Ok(buf)
    } else {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await?;
        Ok(buf)
    }
}

/// Scan the upper directory to find existing overlay files.
async fn scan_upper_dir(base: &Path, dir: &Path, files: &mut HashSet<String>) {
    if let Ok(mut read_dir) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(".overlay-") {
                continue;
            }

            if let Ok(rel) = entry.path().strip_prefix(base) {
                files.insert(rel.to_string_lossy().to_string());
            }

            if entry.path().is_dir() {
                Box::pin(scan_upper_dir(base, &entry.path(), files)).await;
            }
        }
    }
}

/// Whether a path in the deleted-list json is safe to trust.
///
/// Mirrors [`safe_join`] but purely lexical: rejects absolute, null-byte,
/// and traversal-containing entries. Accepted entries never leave the
/// overlay when joined under [`OverlayProvider::upper_path`].
fn is_safe_deleted_entry(p: &str) -> bool {
    if p.is_empty() || p.contains('\0') {
        return false;
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return false;
    }
    for comp in Path::new(p).components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

/// Load the deleted files list from disk.
/// Validates that deserialized paths do not contain traversal sequences.
async fn load_deleted_list(upper_path: &Path) -> HashSet<String> {
    let list_path = upper_path.join(".overlay-deleted.json");
    let data = match tokio::fs::read_to_string(&list_path).await {
        Ok(d) => d,
        Err(_) => return HashSet::new(),
    };

    match serde_json::from_str::<HashSet<String>>(&data) {
        Ok(deleted) => deleted
            .into_iter()
            .filter(|p| {
                let ok = is_safe_deleted_entry(p);
                if !ok {
                    tracing::warn!(path = %p, "overlay: ignoring unsafe deleted-list entry");
                }
                ok
            })
            .collect(),
        Err(e) => {
            // Corrupt file must not crash the mount — just start empty.
            tracing::warn!(error = %e, "failed to parse overlay deleted list; starting empty");
            HashSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_deleted_entry_accepts_relative() {
        assert!(is_safe_deleted_entry("a.txt"));
        assert!(is_safe_deleted_entry("sub/a.txt"));
    }

    #[test]
    fn safe_deleted_entry_rejects_traversal() {
        assert!(!is_safe_deleted_entry(""));
        assert!(!is_safe_deleted_entry("/etc/passwd"));
        assert!(!is_safe_deleted_entry("\\evil"));
        assert!(!is_safe_deleted_entry("../etc/passwd"));
        assert!(!is_safe_deleted_entry("a/../b"));
        assert!(!is_safe_deleted_entry("a\0b"));
    }

    /// Lightweight FileProvider for testing copy_up. Returns small payloads
    /// quickly so many concurrent callers can contend.
    struct StaticLower {
        content: Vec<u8>,
    }

    #[async_trait]
    impl FileProvider for StaticLower {
        async fn list_dir(&self, _path: &str) -> Result<Vec<VfsEntry>, VfsError> {
            Ok(Vec::new())
        }
        async fn stat(&self, _path: &str) -> Result<VfsEntry, VfsError> {
            Err(VfsError::NotFound("test".into()))
        }
        async fn read_file(
            &self,
            _path: &str,
            _offset: u64,
            _length: u64,
        ) -> Result<Vec<u8>, VfsError> {
            Ok(self.content.clone())
        }
    }

    #[tokio::test]
    async fn copy_up_is_serialized_per_path() {
        let tmp = std::env::temp_dir().join(format!("runesh-vfs-overlay-{}", uuid::Uuid::new_v4()));
        let lower = Arc::new(StaticLower {
            content: b"base content".to_vec(),
        });
        let overlay = Arc::new(
            OverlayProvider::new(lower, tmp.clone())
                .await
                .expect("new overlay"),
        );

        // Fire 16 concurrent copy_up calls for the same path. All should
        // converge: exactly one upper file exists with the base content.
        let mut handles = Vec::new();
        for _ in 0..16 {
            let o = Arc::clone(&overlay);
            handles.push(tokio::spawn(async move { o.copy_up("same.txt").await }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }

        assert!(overlay.has_upper("same.txt").await);
        let contents = tokio::fs::read(tmp.join("same.txt")).await.unwrap();
        assert_eq!(contents, b"base content");

        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
