//! Filesystem scanning for backup.

use std::path::{Path, PathBuf};

use crate::{BackupError, content_hash};

/// A file discovered during scanning.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
    pub hash: String,
    pub modified: std::time::SystemTime,
}

/// Scan a directory recursively and return all files with their hashes.
pub async fn scan_directory(root: &Path) -> Result<Vec<ScannedFile>, BackupError> {
    let mut files = Vec::new();
    scan_recursive(root, &mut files).await?;
    Ok(files)
}

#[async_recursion::async_recursion]
async fn scan_recursive(dir: &Path, files: &mut Vec<ScannedFile>) -> Result<(), BackupError> {
    // Use std::fs since tokio::fs::read_dir has lifetime issues with recursion
    let entries = std::fs::read_dir(dir)
        .map_err(|e| BackupError::Storage(format!("read_dir {}: {e}", dir.display())))?;

    for entry in entries {
        let entry = entry.map_err(|e| BackupError::Storage(format!("dir entry: {e}")))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|e| BackupError::Storage(format!("metadata {}: {e}", path.display())))?;

        if metadata.is_dir() {
            scan_recursive(&path, files).await?;
        } else if metadata.is_file() {
            let data = std::fs::read(&path)
                .map_err(|e| BackupError::Storage(format!("read {}: {e}", path.display())))?;
            let hash = content_hash(&data);
            files.push(ScannedFile {
                path,
                size: metadata.len(),
                hash,
                modified: metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(())
}

/// Scan a directory and back it up to a repository, returning the snapshot.
pub async fn backup_directory(
    root: &Path,
    hostname: &str,
    chunk_size: usize,
    repo: &mut crate::BackupRepo,
    tags: Vec<String>,
) -> Result<crate::Snapshot, BackupError> {
    let files = scan_directory(root).await?;

    // Concatenate all file data for chunking
    let mut all_data = Vec::new();
    let mut paths = Vec::new();
    for f in &files {
        let data = std::fs::read(&f.path)
            .map_err(|e| BackupError::Storage(format!("read {}: {e}", f.path.display())))?;
        all_data.extend_from_slice(&data);
        paths.push(f.path.to_string_lossy().to_string());
    }

    let chunks = crate::chunk_data(&all_data, chunk_size);
    let snapshot = repo.create_snapshot(hostname, paths, &chunks, tags);
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn scan_temp_directory() {
        let dir = std::env::temp_dir().join("runesh-backup-scan-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "hello").unwrap();
        fs::write(dir.join("sub/b.txt"), "world").unwrap();

        let files = scan_directory(&dir).await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path.ends_with("a.txt")));
        assert!(files.iter().any(|f| f.path.ends_with("b.txt")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backup_and_restore() {
        let dir = std::env::temp_dir().join("runesh-backup-full-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file1.txt"), "content one").unwrap();
        fs::write(dir.join("file2.txt"), "content two").unwrap();

        let mut repo = crate::BackupRepo::new();
        let snap = backup_directory(&dir, "test-host", 64, &mut repo, vec![])
            .await
            .unwrap();
        assert_eq!(snap.paths.len(), 2);
        assert!(snap.total_size > 0);

        // Restore should produce the concatenated data
        let restored = repo.restore(&snap.id).unwrap();
        let original = String::from_utf8(restored).unwrap();
        assert!(original.contains("content one"));
        assert!(original.contains("content two"));

        let _ = fs::remove_dir_all(&dir);
    }
}
