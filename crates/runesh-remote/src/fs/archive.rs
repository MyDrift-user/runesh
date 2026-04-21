//! Archive creation (zip) for file downloads.

use std::io::Write;
use std::path::Path;

use crate::error::RemoteError;
use crate::fs::security::FsPolicy;

/// Create a zip archive from a list of paths.
/// Returns the path to the created temporary zip file.
pub async fn create_zip_archive(
    policy: &FsPolicy,
    paths: &[String],
) -> Result<std::path::PathBuf, RemoteError> {
    let mut resolved_paths = Vec::new();
    for p in paths {
        resolved_paths.push(policy.resolve_path(p)?);
    }

    let root = policy
        .root
        .canonicalize()
        .map_err(|e| RemoteError::Internal(format!("Failed to canonicalize root: {e}")))?;

    let output_path =
        std::env::temp_dir().join(format!("runesh-archive-{}.zip", uuid::Uuid::new_v4()));
    let output_path_clone = output_path.clone();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&output_path_clone).map_err(RemoteError::Io)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));

        for path in &resolved_paths {
            if path.is_file() {
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                zip.start_file(&relative, options)
                    .map_err(|e| RemoteError::Internal(format!("Zip error: {e}")))?;
                let data = std::fs::read(path)?;
                zip.write_all(&data)?;
            } else if path.is_dir() {
                add_dir_to_zip(&mut zip, path, &root, options)?;
            }
        }

        zip.finish()
            .map_err(|e| RemoteError::Internal(format!("Zip finish error: {e}")))?;

        Ok::<_, RemoteError>(())
    })
    .await
    .map_err(|e| RemoteError::Internal(format!("Archive task failed: {e}")))??;

    Ok(output_path)
}

/// Recursively add a directory to a zip archive.
fn add_dir_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    root: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<(), RemoteError> {
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if path.is_file() {
            zip.start_file(&relative, options)
                .map_err(|e| RemoteError::Internal(format!("Zip error: {e}")))?;
            let data = std::fs::read(path)?;
            zip.write_all(&data)?;
        } else if path.is_dir() && !relative.is_empty() {
            let dir_name = if relative.ends_with('/') {
                relative
            } else {
                format!("{relative}/")
            };
            zip.add_directory(&dir_name, options)
                .map_err(|e| RemoteError::Internal(format!("Zip dir error: {e}")))?;
        }
    }

    Ok(())
}
