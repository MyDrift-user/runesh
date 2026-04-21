//! FileProvider trait — the abstract file content source.
//!
//! Consumer applications implement this trait to supply files from any source:
//! HTTP API, database, S3, local disk, or another VFS.

use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::VfsError;

/// A single file or directory entry in the virtual filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VfsEntry {
    /// File or directory name (e.g., "homework.py").
    pub name: String,
    /// Full path relative to the VFS root (e.g., "assignments/homework.py").
    pub path: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// File size in bytes (0 for directories).
    pub size: u64,
    /// Creation time.
    pub created: Option<SystemTime>,
    /// Last modification time.
    pub modified: Option<SystemTime>,
    /// Last access time.
    pub accessed: Option<SystemTime>,
    /// Whether the file is read-only at the provider level.
    pub readonly: bool,
    /// Whether the file content is available locally (hydrated).
    #[serde(default)]
    pub is_hydrated: bool,
    /// Content hash for change detection (e.g., SHA-256 hex).
    pub content_hash: Option<String>,
    /// MIME type hint (e.g., "application/pdf").
    pub content_type: Option<String>,
    /// Unix owner uid, if the provider tracks it. Falls back to
    /// [`crate::config::MountConfig::default_uid`] (then process uid).
    /// **Multi-tenant deployments MUST provide per-tenant uids here.**
    #[serde(default)]
    pub uid: Option<u32>,
    /// Unix group gid, same fallback chain as `uid`.
    #[serde(default)]
    pub gid: Option<u32>,
}

impl VfsEntry {
    /// Create a directory entry.
    pub fn directory(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: true,
            size: 0,
            created: None,
            modified: Some(SystemTime::now()),
            accessed: None,
            readonly: false,
            is_hydrated: true,
            content_hash: None,
            content_type: None,
            uid: None,
            gid: None,
        }
    }

    /// Create a file entry.
    pub fn file(name: &str, path: &str, size: u64) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            is_dir: false,
            size,
            created: None,
            modified: Some(SystemTime::now()),
            accessed: None,
            readonly: false,
            is_hydrated: false,
            content_hash: None,
            content_type: None,
            uid: None,
            gid: None,
        }
    }
}

/// Abstract file content source — implemented by consumer applications.
///
/// The VFS platform layer calls these methods when the OS requests file data.
/// Implementations can fetch from HTTP APIs, databases, S3, local disk, etc.
///
/// # Example
///
/// ```ignore
/// struct HttpFileProvider { base_url: String }
///
/// #[async_trait]
/// impl FileProvider for HttpFileProvider {
///     async fn list_dir(&self, path: &str) -> Result<Vec<VfsEntry>, VfsError> {
///         let url = format!("{}/api/files?dir={}", self.base_url, path);
///         let entries = reqwest::get(&url).await?.json().await?;
///         Ok(entries)
///     }
///     // ... other methods
/// }
/// ```
#[async_trait]
pub trait FileProvider: Send + Sync + 'static {
    /// List entries in a directory.
    /// `path` is relative to the VFS root (empty string = root directory).
    async fn list_dir(&self, path: &str) -> Result<Vec<VfsEntry>, VfsError>;

    /// Get metadata for a single file or directory.
    async fn stat(&self, path: &str) -> Result<VfsEntry, VfsError>;

    /// Read file content with optional byte range.
    /// `offset` = 0, `length` = 0 means read the entire file.
    async fn read_file(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>, VfsError>;

    /// Write file content at the given offset.
    /// Called only in WriteThrough mode or for Admin roles.
    async fn write_file(&self, path: &str, data: &[u8], offset: u64) -> Result<(), VfsError> {
        let _ = (path, data, offset);
        Err(VfsError::ReadOnly)
    }

    /// Delete a file or empty directory.
    async fn delete(&self, path: &str) -> Result<(), VfsError> {
        let _ = path;
        Err(VfsError::ReadOnly)
    }

    /// Create a directory.
    async fn mkdir(&self, path: &str) -> Result<(), VfsError> {
        let _ = path;
        Err(VfsError::ReadOnly)
    }

    /// Rename or move a file/directory.
    async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), VfsError> {
        let _ = (old_path, new_path);
        Err(VfsError::ReadOnly)
    }

    /// Get the total file size (for progress reporting during hydration).
    async fn file_size(&self, path: &str) -> Result<u64, VfsError> {
        let entry = self.stat(path).await?;
        Ok(entry.size)
    }
}
