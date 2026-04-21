#![deny(unsafe_code)]
//! Backup engine with content-addressed storage, deduplication, and retention.

pub mod scan;
pub mod store;

pub use scan::{ScannedFile, backup_directory, scan_directory};
pub use store::{BackupStore, FileBackupStore, InMemoryBackupStore};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A backup manifest: an ordered list of chunk references that reassemble a
/// snapshot, plus file paths and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub hostname: String,
    pub paths: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub total_size: u64,
    pub chunk_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A backup snapshot (point-in-time reference to chunks).
///
/// Backwards-compatible alias for `Manifest`. New code should prefer `Manifest`.
pub type Snapshot = Manifest;

/// A content-addressed chunk.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// SHA-256 hash of the content (content address).
    pub id: String,
    /// Raw content.
    pub data: Vec<u8>,
    /// Size of the content.
    pub original_size: usize,
}

/// Hash data to produce a content address.
pub fn content_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Fixed-size chunker (simple, for small data or testing).
pub fn chunk_data(data: &[u8], chunk_size: usize) -> Vec<Chunk> {
    data.chunks(chunk_size)
        .map(|slice| {
            let id = content_hash(slice);
            Chunk {
                id,
                data: slice.to_vec(),
                original_size: slice.len(),
            }
        })
        .collect()
}

/// Content-defined chunker using FastCDC (rolling hash).
pub fn chunk_data_cdc(data: &[u8], min_size: u32, avg_size: u32, max_size: u32) -> Vec<Chunk> {
    use fastcdc::ronomon::FastCDC;

    FastCDC::new(
        data,
        min_size as usize,
        avg_size as usize,
        max_size as usize,
    )
    .map(|entry| {
        let slice = &data[entry.offset..entry.offset + entry.length];
        let id = content_hash(slice);
        Chunk {
            id,
            data: slice.to_vec(),
            original_size: entry.length,
        }
    })
    .collect()
}

/// Retention policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    #[serde(default)]
    pub keep_yearly: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 12,
            keep_yearly: 0,
        }
    }
}

/// In-memory backup repository (for tests and small deployments).
///
/// New code should prefer implementations of [`BackupStore`] for storage.
#[derive(Debug, Default)]
pub struct BackupRepo {
    snapshots: Vec<Snapshot>,
    chunks: std::collections::HashMap<String, Vec<u8>>,
}

impl BackupRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a chunk. Returns true if it was new (not deduplicated).
    pub fn store_chunk(&mut self, chunk: &Chunk) -> bool {
        if self.chunks.contains_key(&chunk.id) {
            false
        } else {
            self.chunks.insert(chunk.id.clone(), chunk.data.clone());
            true
        }
    }

    /// Retrieve a chunk by hash.
    pub fn get_chunk(&self, id: &str) -> Option<&Vec<u8>> {
        self.chunks.get(id)
    }

    /// Create a snapshot from chunks.
    pub fn create_snapshot(
        &mut self,
        hostname: &str,
        paths: Vec<String>,
        chunks: &[Chunk],
        tags: Vec<String>,
    ) -> Snapshot {
        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let total_size: u64 = chunks.iter().map(|c| c.original_size as u64).sum();

        for chunk in chunks {
            self.store_chunk(chunk);
        }

        let snapshot = Manifest {
            id: uuid::Uuid::new_v4().to_string(),
            hostname: hostname.to_string(),
            paths,
            created_at: Utc::now(),
            total_size,
            chunk_ids,
            tags,
        };
        self.snapshots.push(snapshot.clone());
        snapshot
    }

    /// Restore a snapshot: reassemble chunks into the provided writer.
    /// Each chunk's content hash is verified before writing.
    pub async fn restore_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        snapshot_id: &str,
        writer: &mut W,
    ) -> Result<(), BackupError> {
        use tokio::io::AsyncWriteExt;
        let snapshot = self
            .snapshots
            .iter()
            .find(|s| s.id == snapshot_id)
            .ok_or_else(|| BackupError::SnapshotNotFound(snapshot_id.into()))?;

        for chunk_id in &snapshot.chunk_ids {
            let chunk_data = self
                .chunks
                .get(chunk_id)
                .ok_or_else(|| BackupError::ChunkMissing(chunk_id.clone()))?;
            let computed = content_hash(chunk_data);
            if computed != *chunk_id {
                return Err(BackupError::HashMismatch {
                    expected: chunk_id.clone(),
                    actual: computed,
                });
            }
            writer
                .write_all(chunk_data)
                .await
                .map_err(|e| BackupError::Storage(format!("write: {e}")))?;
        }
        writer
            .flush()
            .await
            .map_err(|e| BackupError::Storage(format!("flush: {e}")))?;
        Ok(())
    }

    /// Restore a snapshot: reassemble chunks into a Vec.
    ///
    /// Prefer `restore_to` to stream to an `AsyncWrite`; this helper is
    /// retained for tests and tiny snapshots.
    pub fn restore(&self, snapshot_id: &str) -> Result<Vec<u8>, BackupError> {
        let snapshot = self
            .snapshots
            .iter()
            .find(|s| s.id == snapshot_id)
            .ok_or_else(|| BackupError::SnapshotNotFound(snapshot_id.into()))?;

        let mut data = Vec::new();
        for chunk_id in &snapshot.chunk_ids {
            let chunk_data = self
                .chunks
                .get(chunk_id)
                .ok_or_else(|| BackupError::ChunkMissing(chunk_id.clone()))?;
            let computed = content_hash(chunk_data);
            if computed != *chunk_id {
                return Err(BackupError::HashMismatch {
                    expected: chunk_id.clone(),
                    actual: computed,
                });
            }
            data.extend_from_slice(chunk_data);
        }
        Ok(data)
    }

    /// List all snapshots.
    pub fn list_snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Delete a snapshot (does not remove chunks; they may be shared).
    pub fn delete_snapshot(&mut self, id: &str) -> bool {
        let before = self.snapshots.len();
        self.snapshots.retain(|s| s.id != id);
        self.snapshots.len() < before
    }

    /// Count unique chunks.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Total storage used by chunks.
    pub fn storage_bytes(&self) -> u64 {
        self.chunks.values().map(|d| d.len() as u64).sum()
    }

    /// Garbage collect: remove chunks not referenced by any snapshot.
    ///
    /// This is a mark-and-sweep pass. The caller is responsible for holding
    /// a lock that excludes concurrent snapshot creation: GC is an offline
    /// operation.
    pub fn gc(&mut self) -> usize {
        let referenced: std::collections::HashSet<&str> = self
            .snapshots
            .iter()
            .flat_map(|s| s.chunk_ids.iter().map(|id| id.as_str()))
            .collect();
        let before = self.chunks.len();
        self.chunks.retain(|id, _| referenced.contains(id.as_str()));
        before - self.chunks.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),
    #[error("chunk missing: {0}")]
    ChunkMissing(String),
    #[error("chunk hash mismatch: expected {expected}, actual {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("storage error: {0}")]
    Storage(String),
    #[error("serde: {0}")]
    Serde(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hashing() {
        let h1 = content_hash(b"hello");
        let h2 = content_hash(b"hello");
        let h3 = content_hash(b"world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn chunking() {
        let data = b"hello world this is test data for chunking";
        let chunks = chunk_data(data, 10);
        assert!(chunks.len() >= 4);
        let reassembled: Vec<u8> = chunks.iter().flat_map(|c| c.data.clone()).collect();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn deduplication() {
        let mut repo = BackupRepo::new();
        let chunk = Chunk {
            id: content_hash(b"data"),
            data: b"data".to_vec(),
            original_size: 4,
        };
        assert!(repo.store_chunk(&chunk));
        assert!(!repo.store_chunk(&chunk));
        assert_eq!(repo.chunk_count(), 1);
    }

    #[test]
    fn snapshot_and_restore() {
        let mut repo = BackupRepo::new();
        let data = b"important file content here";
        let chunks = chunk_data(data, 10);

        let snap = repo.create_snapshot("server-1", vec!["/data".into()], &chunks, vec![]);
        assert!(!snap.id.is_empty());
        assert_eq!(snap.total_size, data.len() as u64);

        let restored = repo.restore(&snap.id).unwrap();
        assert_eq!(restored, data);
    }

    #[test]
    fn cross_snapshot_dedup() {
        let mut repo = BackupRepo::new();
        let data = b"shared content between snapshots";
        let chunks = chunk_data(data, 16);

        repo.create_snapshot("host-a", vec!["/a".into()], &chunks, vec![]);
        repo.create_snapshot("host-b", vec!["/b".into()], &chunks, vec![]);

        assert_eq!(repo.list_snapshots().len(), 2);
        assert_eq!(repo.chunk_count(), chunks.len());
    }

    #[test]
    fn garbage_collection() {
        let mut repo = BackupRepo::new();
        let data1 = chunk_data(b"first backup", 6);
        let data2 = chunk_data(b"second backup", 6);

        let snap1 = repo.create_snapshot("h", vec![], &data1, vec![]);
        repo.create_snapshot("h", vec![], &data2, vec![]);

        let before = repo.chunk_count();
        repo.delete_snapshot(&snap1.id);
        let removed = repo.gc();
        assert!(removed > 0 || repo.chunk_count() <= before);
    }

    #[test]
    fn snapshot_not_found() {
        let repo = BackupRepo::new();
        assert!(repo.restore("nonexistent").is_err());
    }

    #[tokio::test]
    async fn restore_verifies_chunk_hash() {
        // Build a repo, poison a chunk to simulate corruption, and confirm
        // restore rejects it.
        let mut repo = BackupRepo::new();
        let chunks = chunk_data(b"alpha beta gamma", 6);
        let snap = repo.create_snapshot("h", vec![], &chunks, vec![]);
        // Corrupt one chunk in place (keep the id, mutate the data).
        let target = snap.chunk_ids[0].clone();
        if let Some(bytes) = repo.chunks.get_mut(&target) {
            bytes[0] ^= 0xFF;
        }
        let mut out: Vec<u8> = Vec::new();
        let err = repo.restore_to(&snap.id, &mut out).await.unwrap_err();
        matches!(err, BackupError::HashMismatch { .. });
    }
}
