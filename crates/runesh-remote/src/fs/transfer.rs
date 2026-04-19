//! Chunked file upload and download with progress tracking.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::RemoteError;
use crate::fs::security::FsPolicy;

/// Manages chunked file uploads, assembling chunks into complete files.
#[derive(Clone)]
pub struct UploadManager {
    /// In-progress uploads keyed by path.
    uploads: Arc<RwLock<HashMap<String, UploadState>>>,
    policy: Arc<FsPolicy>,
}

struct UploadState {
    path: PathBuf,
    total_chunks: u32,
    received_chunks: Vec<bool>,
    temp_dir: PathBuf,
    created_at: std::time::Instant,
}

impl UploadManager {
    pub fn new(policy: Arc<FsPolicy>) -> Self {
        Self {
            uploads: Arc::new(RwLock::new(HashMap::new())),
            policy,
        }
    }

    /// Handle an uploaded chunk. Returns (is_complete, percent).
    pub async fn handle_chunk(
        &self,
        path: &str,
        chunk_index: u32,
        total_chunks: u32,
        data: &[u8],
    ) -> Result<(bool, f32), RemoteError> {
        let resolved = self.policy.resolve_path(path)?;
        self.policy.check_write()?;

        let key = path.to_string();

        // Initialize upload state if first chunk
        {
            let mut uploads = self.uploads.write().await;
            if !uploads.contains_key(&key) {
                let temp_dir =
                    std::env::temp_dir().join(format!("runesh-upload-{}", uuid::Uuid::new_v4()));
                tokio::fs::create_dir_all(&temp_dir).await?;

                uploads.insert(
                    key.clone(),
                    UploadState {
                        path: resolved.clone(),
                        total_chunks,
                        received_chunks: vec![false; total_chunks as usize],
                        temp_dir,
                        created_at: std::time::Instant::now(),
                    },
                );
            }
        }

        // Write chunk to temp file
        let (is_complete, percent) = {
            let mut uploads = self.uploads.write().await;
            let state = uploads
                .get_mut(&key)
                .ok_or_else(|| RemoteError::Internal("Upload state lost".into()))?;

            // Bounds check: reject invalid chunk indices
            if chunk_index >= state.total_chunks {
                return Err(RemoteError::BadRequest(format!(
                    "Chunk index {chunk_index} exceeds total chunks {}",
                    state.total_chunks
                )));
            }

            // Reject duplicate chunks
            if state.received_chunks[chunk_index as usize] {
                return Err(RemoteError::BadRequest(format!(
                    "Chunk {chunk_index} already received"
                )));
            }

            let chunk_path = state.temp_dir.join(format!("chunk_{chunk_index:06}"));
            tokio::fs::write(&chunk_path, data).await?;

            state.received_chunks[chunk_index as usize] = true;
            let received = state.received_chunks.iter().filter(|&&r| r).count() as u32;
            let percent = (received as f32 / total_chunks as f32) * 100.0;
            let is_complete = received == total_chunks;

            (is_complete, percent)
        };

        // If all chunks received, assemble the final file
        if is_complete {
            self.assemble_file(&key).await?;
        }

        Ok((is_complete, percent))
    }

    /// Assemble all chunks into the final file.
    async fn assemble_file(&self, key: &str) -> Result<(), RemoteError> {
        let (path, temp_dir, total_chunks) = {
            let uploads = self.uploads.read().await;
            let state = uploads
                .get(key)
                .ok_or_else(|| RemoteError::Internal("Upload state not found".into()))?;
            (
                state.path.clone(),
                state.temp_dir.clone(),
                state.total_chunks,
            )
        };

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Concatenate chunks in order
        use tokio::io::AsyncWriteExt;
        let mut output = tokio::fs::File::create(&path).await?;
        for i in 0..total_chunks {
            let chunk_path = temp_dir.join(format!("chunk_{i:06}"));
            let chunk_data = tokio::fs::read(&chunk_path).await?;
            output.write_all(&chunk_data).await?;
        }
        output.flush().await?;

        // Cleanup temp directory
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        // Remove upload state
        self.uploads.write().await.remove(key);

        tracing::info!(path = %path.display(), "File upload assembled");
        Ok(())
    }

    /// Clean up stale uploads (older than timeout).
    pub async fn cleanup_stale(&self, timeout: std::time::Duration) {
        let mut uploads = self.uploads.write().await;
        let stale_keys: Vec<String> = uploads
            .iter()
            .filter(|(_, state)| state.created_at.elapsed() > timeout)
            .map(|(key, _)| key.clone())
            .collect();

        for key in stale_keys {
            if let Some(state) = uploads.remove(&key) {
                let _ = tokio::fs::remove_dir_all(&state.temp_dir).await;
                tracing::warn!(path = %key, "Cleaned up stale upload");
            }
        }
    }
}

/// Generate download chunks for a file.
pub async fn download_chunks(
    policy: &FsPolicy,
    path: &str,
    chunk_size: usize,
) -> Result<DownloadIterator, RemoteError> {
    let resolved = policy.resolve_path(path)?;

    if !resolved.is_file() {
        return Err(RemoteError::NotFound(format!("Not a file: {}", path)));
    }

    let metadata = tokio::fs::metadata(&resolved).await?;
    let total_size = metadata.len();
    let total_chunks = ((total_size as f64) / (chunk_size as f64)).ceil() as u32;

    Ok(DownloadIterator {
        path: resolved,
        total_size,
        total_chunks,
        chunk_size,
        current_chunk: 0,
    })
}

/// Iterator over file download chunks.
pub struct DownloadIterator {
    path: PathBuf,
    pub total_size: u64,
    pub total_chunks: u32,
    chunk_size: usize,
    current_chunk: u32,
}

impl DownloadIterator {
    /// Read the next chunk. Returns None when all chunks have been read.
    pub async fn next_chunk(&mut self) -> Result<Option<(u32, Vec<u8>)>, RemoteError> {
        if self.current_chunk >= self.total_chunks {
            return Ok(None);
        }

        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let mut file = tokio::fs::File::open(&self.path).await?;
        let offset = (self.current_chunk as u64) * (self.chunk_size as u64);
        file.seek(std::io::SeekFrom::Start(offset)).await?;

        let remaining = (self.total_size - offset) as usize;
        let read_size = remaining.min(self.chunk_size);
        let mut buffer = vec![0u8; read_size];
        file.read_exact(&mut buffer).await?;

        let chunk_index = self.current_chunk;
        self.current_chunk += 1;

        Ok(Some((chunk_index, buffer)))
    }
}
