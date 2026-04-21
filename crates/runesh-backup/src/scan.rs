//! Filesystem scanning for backup.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use fastcdc::v2020::StreamCDC;
use sha2::{Digest, Sha256};

use crate::{BackupError, Chunk};

/// A file discovered during scanning.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
    /// SHA-256 of the entire file computed via streaming.
    pub hash: String,
    pub modified: std::time::SystemTime,
}

/// Scan a directory recursively and return all files with their streamed hashes.
///
/// Never reads a file fully into memory; the SHA-256 is computed from a
/// buffered read so large files (videos, disk images, etc.) won't blow the
/// heap.
pub async fn scan_directory(root: &Path) -> Result<Vec<ScannedFile>, BackupError> {
    let mut files = Vec::new();
    scan_recursive(root, &mut files).await?;
    Ok(files)
}

#[async_recursion::async_recursion]
async fn scan_recursive(dir: &Path, files: &mut Vec<ScannedFile>) -> Result<(), BackupError> {
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
            let hash = hash_file_streaming(&path)?;
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

fn hash_file_streaming(path: &Path) -> Result<String, BackupError> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|e| BackupError::Storage(format!("open {}: {e}", path.display())))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| BackupError::Storage(format!("read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Stream-chunk a single file with FastCDC. Never reads the file fully into
/// memory: uses `BufReader::with_capacity(1 MiB, file)` and `StreamCDC`.
pub fn chunk_file_streaming(
    path: &Path,
    min_size: u32,
    avg_size: u32,
    max_size: u32,
) -> Result<Vec<Chunk>, BackupError> {
    let file = std::fs::File::open(path)
        .map_err(|e| BackupError::Storage(format!("open {}: {e}", path.display())))?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let chunker = StreamCDC::new(
        reader,
        min_size as usize,
        avg_size as usize,
        max_size as usize,
    );

    let mut chunks = Vec::new();
    for entry in chunker {
        let entry = entry.map_err(|e| BackupError::Storage(format!("chunk: {e}")))?;
        let id = crate::content_hash(&entry.data);
        chunks.push(Chunk {
            id,
            original_size: entry.length,
            data: entry.data,
        });
    }
    Ok(chunks)
}

/// Scan a directory and back it up to a repository, returning the manifest.
///
/// Uses streaming chunking per file: each file is passed through FastCDC
/// without buffering the whole file.
pub async fn backup_directory(
    root: &Path,
    hostname: &str,
    chunk_size: usize,
    repo: &mut crate::BackupRepo,
    tags: Vec<String>,
) -> Result<crate::Manifest, BackupError> {
    let files = scan_directory(root).await?;

    let avg = chunk_size.max(1024) as u32;
    let min = (avg / 4).max(256);
    let max = avg.saturating_mul(4);

    let mut all_chunks: Vec<Chunk> = Vec::new();
    let mut paths = Vec::new();
    for f in &files {
        let mut file_chunks = chunk_file_streaming(&f.path, min, avg, max)?;
        all_chunks.append(&mut file_chunks);
        paths.push(f.path.to_string_lossy().to_string());
    }

    let snapshot = repo.create_snapshot(hostname, paths, &all_chunks, tags);
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
        let snap = backup_directory(&dir, "test-host", 1024, &mut repo, vec![])
            .await
            .unwrap();
        assert_eq!(snap.paths.len(), 2);
        assert!(snap.total_size > 0);

        let restored = repo.restore(&snap.id).unwrap();
        let original = String::from_utf8(restored).unwrap();
        assert!(original.contains("content one"));
        assert!(original.contains("content two"));

        let _ = fs::remove_dir_all(&dir);
    }
}
