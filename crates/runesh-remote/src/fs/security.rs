//! File system security: path traversal prevention, sandboxing, permission checks.

use std::path::{Path, PathBuf};

use crate::error::RemoteError;

/// Security policy for file system operations.
#[derive(Debug, Clone)]
pub struct FsPolicy {
    /// Root directory for all operations. All paths are resolved relative to this.
    pub root: PathBuf,
    /// Allow write operations (create, modify, delete).
    pub allow_write: bool,
    /// Allow delete operations.
    pub allow_delete: bool,
    /// Allow executing files.
    pub allow_execute: bool,
    /// Maximum file size for uploads (bytes).
    pub max_upload_size: u64,
    /// Maximum directory depth for recursive operations.
    pub max_depth: usize,
    /// Blocked file extensions (e.g., [".exe", ".bat"]).
    pub blocked_extensions: Vec<String>,
    /// Blocked path patterns (e.g., [".git", "node_modules"]).
    pub blocked_patterns: Vec<String>,
}

impl Default for FsPolicy {
    fn default() -> Self {
        Self {
            root: std::env::current_dir().unwrap_or_else(|_| {
                // Use the system temp dir as a safe fallback instead of filesystem root
                std::env::temp_dir()
            }),
            allow_write: true,
            allow_delete: true,
            allow_execute: false,
            max_upload_size: 100 * 1024 * 1024, // 100 MB
            max_depth: 50,
            blocked_extensions: Vec::new(),
            blocked_patterns: vec![".git".into(), ".env".into()],
        }
    }
}

impl FsPolicy {
    /// Create a read-only policy.
    pub fn read_only(root: PathBuf) -> Self {
        Self {
            root,
            allow_write: false,
            allow_delete: false,
            allow_execute: false,
            ..Default::default()
        }
    }

    /// Resolve and validate a path against the security policy.
    /// Returns the canonicalized absolute path within the sandbox.
    pub fn resolve_path(&self, requested: &str) -> Result<PathBuf, RemoteError> {
        // Reject empty paths
        if requested.is_empty() {
            return Ok(self.root.clone());
        }

        // Reject obviously malicious patterns before any path processing
        if requested.contains("..") {
            return Err(RemoteError::PathTraversal(
                "Path must not contain '..'".into(),
            ));
        }

        // Reject null bytes (path injection)
        if requested.contains('\0') {
            return Err(RemoteError::PathTraversal(
                "Path must not contain null bytes".into(),
            ));
        }

        // Build the full path
        let full_path = if Path::new(requested).is_absolute() {
            PathBuf::from(requested)
        } else {
            self.root.join(requested)
        };

        // Canonicalize to resolve symlinks and normalize
        // For new files that don't exist yet, canonicalize the parent
        let canonical = if full_path.exists() {
            full_path.canonicalize().map_err(|e| {
                RemoteError::Internal(format!("Failed to canonicalize path: {e}"))
            })?
        } else {
            let parent = full_path.parent().ok_or_else(|| {
                RemoteError::BadRequest("Invalid path: no parent directory".into())
            })?;
            let file_name = full_path.file_name().ok_or_else(|| {
                RemoteError::BadRequest("Invalid path: no file name".into())
            })?;
            let canonical_parent = parent.canonicalize().map_err(|e| {
                RemoteError::NotFound(format!("Parent directory not found: {e}"))
            })?;
            canonical_parent.join(file_name)
        };

        // Ensure the canonical path is within the root
        let canonical_root = self.root.canonicalize().map_err(|e| {
            RemoteError::Internal(format!("Failed to canonicalize root: {e}"))
        })?;

        if !canonical.starts_with(&canonical_root) {
            tracing::warn!(
                requested = %requested,
                "Path traversal attempt blocked"
            );
            return Err(RemoteError::PathTraversal(
                "Path escapes sandbox root".into(),
            ));
        }

        // Check blocked patterns
        let path_str = canonical.to_string_lossy();
        for pattern in &self.blocked_patterns {
            if path_str.contains(pattern.as_str()) {
                return Err(RemoteError::NotAllowed(format!(
                    "Access to '{pattern}' paths is blocked"
                )));
            }
        }

        // Check blocked extensions
        if let Some(ext) = canonical.extension() {
            let ext_str = format!(".{}", ext.to_string_lossy());
            for blocked in &self.blocked_extensions {
                if ext_str.eq_ignore_ascii_case(blocked) {
                    return Err(RemoteError::NotAllowed(format!(
                        "File extension '{ext_str}' is blocked"
                    )));
                }
            }
        }

        Ok(canonical)
    }

    /// Check if write operations are allowed.
    pub fn check_write(&self) -> Result<(), RemoteError> {
        if !self.allow_write {
            return Err(RemoteError::NotAllowed(
                "Write operations are disabled".into(),
            ));
        }
        Ok(())
    }

    /// Check if delete operations are allowed.
    pub fn check_delete(&self) -> Result<(), RemoteError> {
        if !self.allow_delete {
            return Err(RemoteError::NotAllowed(
                "Delete operations are disabled".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_traversal_blocked() {
        let policy = FsPolicy {
            root: PathBuf::from("/tmp/sandbox"),
            ..Default::default()
        };

        assert!(policy.resolve_path("../../etc/passwd").is_err());
        assert!(policy.resolve_path("foo/../../bar").is_err());
        assert!(policy.resolve_path("..").is_err());
    }

    #[test]
    fn test_null_byte_blocked() {
        let policy = FsPolicy {
            root: PathBuf::from("/tmp/sandbox"),
            ..Default::default()
        };

        assert!(policy.resolve_path("file\0.txt").is_err());
    }

    #[test]
    fn test_read_only_blocks_write() {
        let policy = FsPolicy::read_only(PathBuf::from("/tmp"));
        assert!(policy.check_write().is_err());
        assert!(policy.check_delete().is_err());
    }
}
