//! Multipart file upload helpers for Axum.

use crate::error::AppError;

/// Metadata about an uploaded file.
pub struct UploadedFile {
    /// Original filename from the client.
    pub filename: String,
    /// MIME type (from Content-Type header or guessed).
    pub content_type: String,
    /// File size in bytes.
    pub size: u64,
    /// Where the file was saved on disk.
    pub storage_path: std::path::PathBuf,
    /// Storage key (UUID-based filename for deduplication).
    pub storage_key: String,
}

/// Save an uploaded file to disk with a UUID-based name.
///
/// Returns metadata about the saved file. The file is written to
/// `{storage_dir}/{uuid}.{extension}`.
#[cfg(feature = "axum")]
pub async fn save_upload(
    field: axum::extract::multipart::Field<'_>,
    storage_dir: &std::path::Path,
    max_size: u64,
) -> Result<UploadedFile, AppError> {
    let filename = field
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "upload".to_string());

    let content_type = field
        .content_type()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Generate UUID-based storage key, preserving extension
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let storage_key = if ext.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        format!("{}.{}", uuid::Uuid::new_v4(), ext)
    };

    let storage_path = storage_dir.join(&storage_key);

    // Ensure directory exists
    tokio::fs::create_dir_all(storage_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create storage dir: {e}")))?;

    // Stream to disk
    let data = field
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read upload: {e}")))?;

    if data.len() as u64 > max_size {
        return Err(AppError::BadRequest(format!(
            "File too large: {} bytes (max {})",
            data.len(),
            max_size
        )));
    }

    tokio::fs::write(&storage_path, &data)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    Ok(UploadedFile {
        size: data.len() as u64,
        filename,
        content_type,
        storage_path,
        storage_key,
    })
}
