//! Multipart file upload helpers for Axum.

use crate::error::AppError;

/// Metadata about an uploaded file.
pub struct UploadedFile {
    /// Original filename from the client.
    pub filename: String,
    /// MIME type (from Content-Type header -- should be validated by caller).
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
/// - `storage_dir`: directory to save files in
/// - `max_size`: maximum file size in bytes
/// - `allowed_extensions`: optional allowlist of extensions (e.g. `&["jpg", "png", "pdf"]`).
///   Pass `None` to allow all extensions.
///
/// SECURITY: The file extension is validated against the allowlist if provided.
/// The content-type comes from the client and should NOT be trusted for
/// security decisions. Use the `infer` crate to verify file type from magic bytes
/// if serving files back to browsers.
#[cfg(feature = "axum")]
pub async fn save_upload(
    field: axum::extract::multipart::Field<'_>,
    storage_dir: &std::path::Path,
    max_size: u64,
    allowed_extensions: Option<&[&str]>,
) -> Result<UploadedFile, AppError> {
    let filename = field
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "upload".to_string());

    let content_type = field
        .content_type()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Extract and validate extension
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    if let Some(allowed) = allowed_extensions {
        if !ext.is_empty() && !allowed.iter().any(|a| a.eq_ignore_ascii_case(ext)) {
            return Err(AppError::BadRequest(format!(
                "File extension '.{}' is not allowed. Allowed: {}",
                ext,
                allowed.join(", ")
            )));
        }
    }

    let storage_key = if ext.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        format!("{}.{}", uuid::Uuid::new_v4(), ext.to_ascii_lowercase())
    };

    let storage_path = storage_dir.join(&storage_key);

    // Ensure directory exists
    tokio::fs::create_dir_all(storage_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create storage dir: {e}")))?;

    // Read with size limit enforcement
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
