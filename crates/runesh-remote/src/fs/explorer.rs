//! Core file system operations: list, stat, read, write, copy, move, delete.

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::error::RemoteError;
use crate::fs::security::FsPolicy;
use crate::protocol::{FileEntry, FilePermissions};

/// List directory contents.
pub async fn list_dir(
    policy: &FsPolicy,
    path: &str,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, RemoteError> {
    let resolved = policy.resolve_path(path)?;

    if !resolved.is_dir() {
        return Err(RemoteError::BadRequest(format!(
            "Not a directory: {}",
            path
        )));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&resolved).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files if not requested
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        match file_entry_from_path(&entry.path()).await {
            Ok(fe) => entries.push(fe),
            Err(e) => {
                tracing::debug!(path = %entry.path().display(), error = %e, "Skipping inaccessible entry");
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

/// Get file/directory metadata.
pub async fn stat(policy: &FsPolicy, path: &str) -> Result<FileEntry, RemoteError> {
    let resolved = policy.resolve_path(path)?;
    file_entry_from_path(&resolved).await
}

/// Read file content. Returns (data, total_size, sha256_hex).
pub async fn read_file(
    policy: &FsPolicy,
    path: &str,
    offset: u64,
    length: u64,
) -> Result<(Vec<u8>, u64, String), RemoteError> {
    let resolved = policy.resolve_path(path)?;

    if !resolved.is_file() {
        return Err(RemoteError::BadRequest(format!("Not a file: {}", path)));
    }

    let metadata = tokio::fs::metadata(&resolved).await?;
    let total_size = metadata.len();

    let mut file = tokio::fs::File::open(&resolved).await?;

    if offset > 0 {
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    let read_len = if length == 0 {
        (total_size - offset) as usize
    } else {
        length as usize
    };

    // Cap read size at 10 MB per request
    let read_len = read_len.min(10 * 1024 * 1024);
    let mut buffer = vec![0u8; read_len];
    let bytes_read = file.read(&mut buffer).await?;
    buffer.truncate(bytes_read);

    // Compute checksum
    let mut hasher = Sha256::new();
    hasher.update(&buffer);
    // sha2 0.11 returns hybrid_array::Array; convert via hex::encode which
    // takes AsRef<[u8]>.
    let checksum = hex::encode(hasher.finalize());

    Ok((buffer, total_size, checksum))
}

/// Write data to a file.
pub async fn write_file(
    policy: &FsPolicy,
    path: &str,
    data: &[u8],
    append: bool,
) -> Result<u64, RemoteError> {
    policy.check_write()?;
    let resolved = policy.resolve_path(path)?;

    if append {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&resolved)
            .await?;
        file.write_all(data).await?;
    } else {
        tokio::fs::write(&resolved, data).await?;
    }

    Ok(data.len() as u64)
}

/// Create a directory.
pub async fn mkdir(
    policy: &FsPolicy,
    path: &str,
    recursive: bool,
) -> Result<(), RemoteError> {
    policy.check_write()?;
    let resolved = policy.resolve_path(path)?;

    if recursive {
        tokio::fs::create_dir_all(&resolved).await?;
    } else {
        tokio::fs::create_dir(&resolved).await?;
    }

    Ok(())
}

/// Delete a file or directory.
pub async fn delete(
    policy: &FsPolicy,
    path: &str,
    recursive: bool,
) -> Result<(), RemoteError> {
    policy.check_delete()?;
    let resolved = policy.resolve_path(path)?;

    if resolved.is_dir() {
        if recursive {
            tokio::fs::remove_dir_all(&resolved).await?;
        } else {
            tokio::fs::remove_dir(&resolved).await?;
        }
    } else {
        tokio::fs::remove_file(&resolved).await?;
    }

    Ok(())
}

/// Copy a file or directory.
pub async fn copy(
    policy: &FsPolicy,
    src: &str,
    dst: &str,
) -> Result<(), RemoteError> {
    policy.check_write()?;
    let src_path = policy.resolve_path(src)?;
    let dst_path = policy.resolve_path(dst)?;

    if src_path.is_dir() {
        copy_dir_recursive(&src_path, &dst_path).await?;
    } else {
        tokio::fs::copy(&src_path, &dst_path).await?;
    }

    Ok(())
}

/// Move/rename a file or directory.
pub async fn rename(
    policy: &FsPolicy,
    src: &str,
    dst: &str,
) -> Result<(), RemoteError> {
    policy.check_write()?;
    let src_path = policy.resolve_path(src)?;
    let dst_path = policy.resolve_path(dst)?;

    tokio::fs::rename(&src_path, &dst_path).await?;
    Ok(())
}

/// Search for files matching a glob pattern.
pub async fn search(
    policy: &FsPolicy,
    path: &str,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<FileEntry>, RemoteError> {
    let resolved = policy.resolve_path(path)?;
    let pattern = pattern.to_lowercase();
    let max_results = max_results.min(10_000);

    let resolved_clone = resolved.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(&resolved_clone)
            .max_depth(20)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if results.len() >= max_results {
                break;
            }

            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains(&pattern) {
                results.push(entry.path().to_path_buf());
            }
        }
        results
    })
    .await
    .map_err(|e| RemoteError::Internal(format!("Search task failed: {e}")))?;

    let mut file_entries = Vec::new();
    for path in entries {
        if let Ok(fe) = file_entry_from_path(&path).await {
            file_entries.push(fe);
        }
    }

    Ok(file_entries)
}

/// Build a FileEntry from a path.
async fn file_entry_from_path(path: &Path) -> Result<FileEntry, RemoteError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;

    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        })
        .flatten()
        .map(|dt| dt.to_rfc3339());

    let created = metadata
        .created()
        .ok()
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        })
        .flatten()
        .map(|dt| dt.to_rfc3339());

    let permissions = FilePermissions {
        readonly: metadata.permissions().readonly(),
        mode: get_unix_mode(&metadata),
        hidden: is_hidden(path),
        system: false,
    };

    Ok(FileEntry {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: path.to_string_lossy().to_string(),
        is_dir: metadata.is_dir(),
        is_file: metadata.is_file(),
        is_symlink: metadata.is_symlink(),
        size: metadata.len(),
        modified,
        created,
        permissions,
    })
}

#[cfg(unix)]
fn get_unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn get_unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

/// Recursively copy a directory.
async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), RemoteError> {
    tokio::fs::create_dir_all(dst).await?;

    let mut read_dir = tokio::fs::read_dir(src).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path)).await?;
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }

    Ok(())
}
