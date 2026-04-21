//! Chunked file upload and download with progress tracking.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::RemoteError;
use crate::fs::security::FsPolicy;

/// Hard cap on the number of chunks a single upload may have.
pub const MAX_UPLOAD_CHUNKS: u32 = 10_000;
/// Hard cap on total upload size: 5 GiB.
pub const MAX_UPLOAD_BYTES: u64 = 5 * 1024 * 1024 * 1024;
/// Uploads abandoned for this long are garbage-collected.
pub const UPLOAD_IDLE_TIMEOUT_SECS: u64 = 30 * 60;

/// Manages chunked file uploads, assembling chunks into complete files.
///
/// Uploads are keyed by a **server-assigned UUID** (never by the client-
/// supplied path) and the temp directory for chunks is always under
/// `<tmp>/runesh-uploads/<uuid>/`. Total chunk count and total byte count
/// are both capped at load time to prevent resource exhaustion.
#[derive(Clone)]
pub struct UploadManager {
    /// In-progress uploads keyed by upload_id (UUID string).
    uploads: Arc<RwLock<HashMap<String, UploadState>>>,
    policy: Arc<FsPolicy>,
}

struct UploadState {
    path: PathBuf,
    total_chunks: u32,
    total_size_bytes: u64,
    received_bytes: u64,
    received_chunks: Vec<bool>,
    temp_dir: PathBuf,
    created_at: std::time::Instant,
}

/// Handle returned when a new upload begins.
#[derive(Debug, Clone)]
pub struct UploadHandle {
    pub upload_id: String,
}

impl UploadManager {
    pub fn new(policy: Arc<FsPolicy>) -> Self {
        Self {
            uploads: Arc::new(RwLock::new(HashMap::new())),
            policy,
        }
    }

    /// Begin a new upload. Returns a server-assigned upload id.
    ///
    /// The destination `path` is validated and writability is checked up
    /// front. `total_chunks` and `total_size_bytes` are capped.
    pub async fn begin(
        &self,
        path: &str,
        total_chunks: u32,
        total_size_bytes: u64,
    ) -> Result<UploadHandle, RemoteError> {
        self.policy.check_write()?;
        let resolved = self.policy.resolve_path(path)?;

        if total_chunks == 0 || total_chunks > MAX_UPLOAD_CHUNKS {
            return Err(RemoteError::BadRequest(format!(
                "total_chunks must be in 1..={MAX_UPLOAD_CHUNKS}"
            )));
        }
        if total_size_bytes > MAX_UPLOAD_BYTES {
            return Err(RemoteError::BadRequest(format!(
                "total_size_bytes exceeds {MAX_UPLOAD_BYTES}"
            )));
        }
        if total_size_bytes > self.policy.max_upload_size {
            return Err(RemoteError::NotAllowed(format!(
                "total_size_bytes exceeds policy max_upload_size ({})",
                self.policy.max_upload_size
            )));
        }

        let upload_id = uuid::Uuid::new_v4().to_string();
        let temp_root = std::env::temp_dir().join("runesh-uploads");
        let temp_dir = temp_root.join(&upload_id);
        tokio::fs::create_dir_all(&temp_dir).await?;

        self.uploads.write().await.insert(
            upload_id.clone(),
            UploadState {
                path: resolved,
                total_chunks,
                total_size_bytes,
                received_bytes: 0,
                received_chunks: vec![false; total_chunks as usize],
                temp_dir,
                created_at: std::time::Instant::now(),
            },
        );

        Ok(UploadHandle { upload_id })
    }

    /// Handle an uploaded chunk keyed by upload id. Returns (is_complete, percent).
    pub async fn handle_chunk_by_id(
        &self,
        upload_id: &str,
        chunk_index: u32,
        data: &[u8],
    ) -> Result<(bool, f32), RemoteError> {
        self.policy.check_write()?;

        let (is_complete, percent) = {
            let mut uploads = self.uploads.write().await;
            let state = uploads.get_mut(upload_id).ok_or_else(|| {
                RemoteError::BadRequest(format!("unknown upload id: {upload_id}"))
            })?;

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

            // Size ceiling per upload
            let new_total = state
                .received_bytes
                .checked_add(data.len() as u64)
                .ok_or_else(|| RemoteError::BadRequest("received_bytes overflow".into()))?;
            if new_total > state.total_size_bytes {
                return Err(RemoteError::BadRequest(
                    "chunk exceeds declared total_size_bytes".into(),
                ));
            }
            state.received_bytes = new_total;

            let chunk_path = state.temp_dir.join(format!("chunk_{chunk_index:06}"));
            tokio::fs::write(&chunk_path, data).await?;

            state.received_chunks[chunk_index as usize] = true;
            let received = state.received_chunks.iter().filter(|&&r| r).count() as u32;
            let percent = (received as f32 / state.total_chunks as f32) * 100.0;
            let is_complete = received == state.total_chunks;

            (is_complete, percent)
        };

        if is_complete {
            self.assemble_file(upload_id).await?;
        }

        Ok((is_complete, percent))
    }

    /// Backwards-compatible chunk handler keyed by client-supplied path.
    ///
    /// Synthesizes an upload id the first time it sees a given path so old
    /// callers keep working, but new code should call
    /// [`UploadManager::begin`] + [`UploadManager::handle_chunk_by_id`].
    pub async fn handle_chunk(
        &self,
        path: &str,
        chunk_index: u32,
        total_chunks: u32,
        data: &[u8],
    ) -> Result<(bool, f32), RemoteError> {
        self.policy.check_write()?;

        if total_chunks == 0 || total_chunks > MAX_UPLOAD_CHUNKS {
            return Err(RemoteError::BadRequest(format!(
                "total_chunks must be in 1..={MAX_UPLOAD_CHUNKS}"
            )));
        }

        // Find or create an upload keyed by the resolved path.
        let resolved = self.policy.resolve_path(path)?;
        let upload_id = {
            let uploads = self.uploads.read().await;
            uploads
                .iter()
                .find(|(_, s)| s.path == resolved)
                .map(|(id, _)| id.clone())
        };

        let upload_id = match upload_id {
            Some(id) => id,
            None => {
                // Conservative default cap when we have no declared total
                // size: require chunk index count * policy.max_upload_size.
                let declared_size = self.policy.max_upload_size;
                self.begin(path, total_chunks, declared_size)
                    .await?
                    .upload_id
            }
        };

        self.handle_chunk_by_id(&upload_id, chunk_index, data).await
    }

    /// Assemble all chunks into the final file.
    async fn assemble_file(&self, upload_id: &str) -> Result<(), RemoteError> {
        let (path, temp_dir, total_chunks) = {
            let uploads = self.uploads.read().await;
            let state = uploads.get(upload_id).ok_or_else(|| {
                RemoteError::Internal(format!("upload state not found for id {upload_id}"))
            })?;
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
        self.uploads.write().await.remove(upload_id);

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
                tracing::warn!(upload_id = %key, "Cleaned up stale upload");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager() -> (UploadManager, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "runesh-upload-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut policy = FsPolicy {
            root: dir.clone(),
            max_upload_size: 1024 * 1024,
            ..Default::default()
        };
        policy.allow_write = true;
        (UploadManager::new(Arc::new(policy)), dir)
    }

    #[tokio::test]
    async fn begin_rejects_too_many_chunks() {
        let (mgr, dir) = make_manager();
        let res = mgr.begin("file.bin", MAX_UPLOAD_CHUNKS + 1, 1024).await;
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn begin_rejects_oversized_upload() {
        let (mgr, dir) = make_manager();
        let res = mgr.begin("file.bin", 1, MAX_UPLOAD_BYTES + 1).await;
        assert!(res.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upload_id_is_scoped_and_unique() {
        let (mgr, dir) = make_manager();
        let h1 = mgr.begin("a.bin", 1, 10).await.unwrap();
        let h2 = mgr.begin("b.bin", 1, 10).await.unwrap();
        assert_ne!(h1.upload_id, h2.upload_id);
        // Unknown upload id is rejected.
        let bad = mgr.handle_chunk_by_id("not-a-real-id", 0, &[1, 2, 3]).await;
        assert!(bad.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
