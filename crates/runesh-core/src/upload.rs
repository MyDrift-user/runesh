//! Multipart file upload helpers for Axum.

use crate::error::AppError;

// ── Magic bytes validation ──────────────────────────────────────────────────

/// Known file signatures (magic bytes) for common file types.
///
/// SVG is intentionally NOT in this list. SVG is an XML format that can embed
/// `<script>` tags and event handlers, making it a stored-XSS vector when
/// served back to a browser. Callers who need SVG must opt in explicitly via
/// their allowlist AND serve SVGs with `Content-Disposition: attachment` so
/// the browser downloads instead of rendering them.
const MAGIC_BYTES: &[(&str, &[u8])] = &[
    ("jpg", &[0xFF, 0xD8, 0xFF]),
    ("jpeg", &[0xFF, 0xD8, 0xFF]),
    ("png", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
    ("gif", &[0x47, 0x49, 0x46, 0x38]),
    ("webp", &[0x52, 0x49, 0x46, 0x46]), // RIFF header (also check WEBP at offset 8)
    ("pdf", &[0x25, 0x50, 0x44, 0x46]),
    ("zip", &[0x50, 0x4B, 0x03, 0x04]),
];

/// Default safe upload allowlist — media + documents, no executable / active
/// content. Use this when your application accepts user uploads that may be
/// served back to browsers.
///
/// Deliberately excludes:
/// - `svg` (stored XSS via embedded `<script>`)
/// - `html`, `htm`, `xml`, `xhtml` (stored XSS)
/// - `js`, `mjs`, `css` (script/style injection)
/// - `exe`, `dll`, `so`, `bat`, `cmd`, `sh`, `ps1`, `py`, `php`, `jsp` (RCE)
/// - `svgz`, `swf` (active content)
pub const SAFE_UPLOAD_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "pdf", "txt", "md", "csv", "json", "mp3", "mp4", "webm",
    "ogg", "wav", "zip",
];

/// Validate that file contents match the claimed extension by checking magic bytes.
///
/// Returns `Ok(())` if the magic bytes match the extension, or if the extension
/// is not in the known signatures list (unknown types pass through).
/// Returns `Err` if the magic bytes contradict the claimed extension.
pub fn validate_magic_bytes(data: &[u8], extension: &str) -> Result<(), AppError> {
    let ext = extension.to_ascii_lowercase();

    // Find expected magic bytes for this extension
    let expected: Vec<&[u8]> = MAGIC_BYTES
        .iter()
        .filter(|(e, _)| *e == ext)
        .map(|(_, magic)| *magic)
        .collect();

    // If we don't have signatures for this extension, allow it
    if expected.is_empty() {
        return Ok(());
    }

    // Check if the file starts with any of the expected signatures
    let matches = expected
        .iter()
        .any(|magic| data.len() >= magic.len() && data[..magic.len()] == **magic);

    if !matches {
        return Err(AppError::BadRequest(format!(
            "File content does not match .{ext} format (magic bytes mismatch)"
        )));
    }

    Ok(())
}

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
/// SECURITY:
/// - The file extension is validated against the allowlist if provided.
/// - The client-supplied `Content-Type` is NEVER trusted; type is determined
///   by magic-byte inspection on the first 512 bytes BEFORE any bytes are
///   written to the final storage path.
/// - Bytes are streamed to a `.tmp-<uuid>` sibling file and only renamed to
///   the final `storage_key` after the complete write + a second magic-byte
///   validation pass on the persisted buffer. A handler crash after the
///   initial validation leaves only the temp file, which the caller can
///   clean with a periodic sweep of `.tmp-*` files older than N minutes.
#[cfg(feature = "axum")]
pub async fn save_upload(
    mut field: axum::extract::multipart::Field<'_>,
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
        // Reject files with no extension OR disallowed extensions
        if ext.is_empty() || !allowed.iter().any(|a| a.eq_ignore_ascii_case(ext)) {
            return Err(AppError::BadRequest(format!(
                "File must have an allowed extension. Allowed: {}",
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
    let temp_path = storage_dir.join(format!(".tmp-{}-{}", uuid::Uuid::new_v4(), storage_key));

    // Ensure directory exists
    tokio::fs::create_dir_all(storage_dir)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create storage dir: {e}")))?;

    // Buffer the first 512 bytes in memory and validate BEFORE opening a file
    // on disk. This way a crash during upload never leaves an unvalidated file
    // at the final storage path.
    const MAGIC_BUFFER: usize = 512;
    let needs_magic_check = !ext.is_empty();
    let mut preamble: Vec<u8> = Vec::with_capacity(MAGIC_BUFFER);
    let mut trailing_chunks: Vec<bytes::Bytes> = Vec::new();
    let mut total_size: u64 = 0;

    while preamble.len() < MAGIC_BUFFER {
        let chunk = match field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read upload: {e}")))?
        {
            Some(c) => c,
            None => break,
        };
        total_size += chunk.len() as u64;
        if total_size > max_size {
            return Err(AppError::BadRequest(format!(
                "File too large: exceeds max {} bytes",
                max_size
            )));
        }
        let room = MAGIC_BUFFER - preamble.len();
        if chunk.len() <= room {
            preamble.extend_from_slice(&chunk);
        } else {
            preamble.extend_from_slice(&chunk[..room]);
            trailing_chunks.push(chunk.slice(room..));
        }
    }

    if needs_magic_check {
        // Validate BEFORE touching disk.
        validate_magic_bytes(&preamble, ext)?;
    }

    // Now stream to a `.tmp-*` file, rename only after a successful close + second check.
    let cleanup_on_err = |path: std::path::PathBuf| async move {
        let _ = tokio::fs::remove_file(&path).await;
    };

    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            return Err(AppError::Internal(format!("Failed to create file: {e}")));
        }
    };

    use tokio::io::AsyncWriteExt;

    if let Err(e) = file.write_all(&preamble).await {
        drop(file);
        cleanup_on_err(temp_path.clone()).await;
        return Err(AppError::Internal(format!("Failed to write file: {e}")));
    }
    for chunk in trailing_chunks.drain(..) {
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            cleanup_on_err(temp_path.clone()).await;
            return Err(AppError::Internal(format!("Failed to write file: {e}")));
        }
    }

    // Drain the remainder of the stream.
    while let Some(chunk) = match field.chunk().await {
        Ok(c) => c,
        Err(e) => {
            drop(file);
            cleanup_on_err(temp_path.clone()).await;
            return Err(AppError::BadRequest(format!("Failed to read upload: {e}")));
        }
    } {
        total_size += chunk.len() as u64;
        if total_size > max_size {
            drop(file);
            cleanup_on_err(temp_path.clone()).await;
            return Err(AppError::BadRequest(format!(
                "File too large: exceeds max {} bytes",
                max_size
            )));
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            cleanup_on_err(temp_path.clone()).await;
            return Err(AppError::Internal(format!("Failed to write file: {e}")));
        }
    }

    if let Err(e) = file.flush().await {
        drop(file);
        cleanup_on_err(temp_path.clone()).await;
        return Err(AppError::Internal(format!("Failed to flush file: {e}")));
    }
    drop(file);

    // Second validation pass on the persisted temp file, against tampering
    // in flight (shouldn't happen over HTTPS but defense in depth is cheap).
    if needs_magic_check {
        use tokio::io::AsyncReadExt;
        let mut reopened = match tokio::fs::File::open(&temp_path).await {
            Ok(f) => f,
            Err(e) => {
                cleanup_on_err(temp_path.clone()).await;
                return Err(AppError::Internal(format!("Failed to reopen file: {e}")));
            }
        };
        let mut verify_buf = vec![0u8; MAGIC_BUFFER];
        let n = reopened.read(&mut verify_buf).await.map_err(|e| {
            AppError::Internal(format!("Failed to re-read file for validation: {e}"))
        })?;
        verify_buf.truncate(n);
        if let Err(e) = validate_magic_bytes(&verify_buf, ext) {
            drop(reopened);
            cleanup_on_err(temp_path.clone()).await;
            return Err(e);
        }
    }

    // Atomic rename into the final destination.
    if let Err(e) = tokio::fs::rename(&temp_path, &storage_path).await {
        cleanup_on_err(temp_path.clone()).await;
        return Err(AppError::Internal(format!("Failed to commit upload: {e}")));
    }

    Ok(UploadedFile {
        size: total_size,
        filename,
        content_type,
        storage_path,
        storage_key,
    })
}
