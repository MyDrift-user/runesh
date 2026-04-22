//! Pluggable backup storage backends.
//!
//! Two implementations are shipped:
//!
//! - [`InMemoryBackupStore`] -- fast, unit-test friendly, not persistent.
//! - [`FileBackupStore`] -- persistent, zstd-compressed chunks sharded by the
//!   first two hex chars of the SHA-256, manifests stored as JSON.
//!
//! Chunk layout: `{root}/chunks/{sha[..2]}/{sha}.zst`
//! Manifest layout: `{root}/manifests/{name}.json`
//!
//! Garbage collection is mark-and-sweep. Because GC walks all manifests and
//! then deletes unreferenced chunks, it must not race snapshot creation. The
//! trait exposes a `gc_lock` that callers must acquire for the duration of
//! GC; callers are responsible for treating GC as an offline operation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::{BackupError, Manifest, content_hash};

/// Trait for persistent backup storage.
#[async_trait]
pub trait BackupStore: Send + Sync {
    /// Store a chunk. Implementations must be idempotent: re-storing the same
    /// hash is a no-op.
    async fn put_chunk(&self, hash: &str, data: &[u8]) -> Result<(), BackupError>;

    /// Fetch a chunk by its content hash. Implementations MUST verify the
    /// hash matches before returning.
    async fn get_chunk(&self, hash: &str) -> Result<Vec<u8>, BackupError>;

    /// Check existence without fetching.
    async fn has_chunk(&self, hash: &str) -> Result<bool, BackupError>;

    /// Persist a named manifest.
    async fn write_manifest(&self, name: &str, manifest: &Manifest) -> Result<(), BackupError>;

    /// Load a named manifest.
    async fn read_manifest(&self, name: &str) -> Result<Manifest, BackupError>;

    /// List all manifest names.
    async fn list_manifests(&self) -> Result<Vec<String>, BackupError>;

    /// Delete a named manifest.
    async fn delete_manifest(&self, name: &str) -> Result<(), BackupError>;

    /// Delete a chunk. Called only by `gc`.
    async fn delete_chunk(&self, hash: &str) -> Result<(), BackupError>;

    /// List all chunk hashes. Called only by `gc`.
    async fn list_chunks(&self) -> Result<Vec<String>, BackupError>;
}

/// Mark-and-sweep garbage collector.
///
/// Collects all chunk references from every manifest in the store and deletes
/// any stored chunks that are not referenced. Returns the number of chunks
/// removed. The caller MUST hold exclusive access (e.g., no concurrent
/// snapshot creation) for the duration. Offline operation.
pub async fn gc_offline<S: BackupStore + ?Sized>(store: &S) -> Result<usize, BackupError> {
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in store.list_manifests().await? {
        let manifest = store.read_manifest(&name).await?;
        for h in manifest.chunk_ids {
            referenced.insert(h);
        }
    }

    let mut removed = 0;
    for chunk in store.list_chunks().await? {
        if !referenced.contains(&chunk) {
            store.delete_chunk(&chunk).await?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Apply a retention policy against a [`BackupStore`]: delete manifests
/// that fall outside the GFS buckets, then run [`gc_offline`] so now-
/// unreferenced chunks are pruned. Returns `(manifests_removed,
/// chunks_removed)`. Offline operation; caller must hold exclusive access.
pub async fn apply_retention_offline<S: BackupStore + ?Sized>(
    store: &S,
    policy: &crate::RetentionPolicy,
) -> Result<(usize, usize), BackupError> {
    let names = store.list_manifests().await?;
    let mut manifests: Vec<crate::Manifest> = Vec::with_capacity(names.len());
    // Map manifest IDs back to store names so we can delete by name.
    let mut id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(names.len());
    for name in &names {
        let m = store.read_manifest(name).await?;
        id_to_name.insert(m.id.clone(), name.clone());
        manifests.push(m);
    }

    let to_delete_ids = policy.select_delete(&manifests);
    let mut manifests_removed = 0;
    for id in to_delete_ids {
        if let Some(name) = id_to_name.get(&id) {
            store.delete_manifest(name).await?;
            manifests_removed += 1;
        }
    }

    let chunks_removed = gc_offline(store).await?;
    Ok((manifests_removed, chunks_removed))
}

/// In-memory implementation (for tests).
#[derive(Debug, Default)]
pub struct InMemoryBackupStore {
    inner: Mutex<InMemoryInner>,
}

#[derive(Debug, Default)]
struct InMemoryInner {
    chunks: std::collections::HashMap<String, Vec<u8>>,
    manifests: std::collections::HashMap<String, Manifest>,
}

impl InMemoryBackupStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BackupStore for InMemoryBackupStore {
    async fn put_chunk(&self, hash: &str, data: &[u8]) -> Result<(), BackupError> {
        let mut g = self.inner.lock().await;
        g.chunks.insert(hash.to_string(), data.to_vec());
        Ok(())
    }

    async fn get_chunk(&self, hash: &str) -> Result<Vec<u8>, BackupError> {
        let g = self.inner.lock().await;
        let data = g
            .chunks
            .get(hash)
            .cloned()
            .ok_or_else(|| BackupError::ChunkMissing(hash.to_string()))?;
        let actual = content_hash(&data);
        if actual != hash {
            return Err(BackupError::HashMismatch {
                expected: hash.to_string(),
                actual,
            });
        }
        Ok(data)
    }

    async fn has_chunk(&self, hash: &str) -> Result<bool, BackupError> {
        Ok(self.inner.lock().await.chunks.contains_key(hash))
    }

    async fn write_manifest(&self, name: &str, manifest: &Manifest) -> Result<(), BackupError> {
        let mut g = self.inner.lock().await;
        g.manifests.insert(name.to_string(), manifest.clone());
        Ok(())
    }

    async fn read_manifest(&self, name: &str) -> Result<Manifest, BackupError> {
        let g = self.inner.lock().await;
        g.manifests
            .get(name)
            .cloned()
            .ok_or_else(|| BackupError::SnapshotNotFound(name.to_string()))
    }

    async fn list_manifests(&self) -> Result<Vec<String>, BackupError> {
        Ok(self.inner.lock().await.manifests.keys().cloned().collect())
    }

    async fn delete_manifest(&self, name: &str) -> Result<(), BackupError> {
        self.inner.lock().await.manifests.remove(name);
        Ok(())
    }

    async fn delete_chunk(&self, hash: &str) -> Result<(), BackupError> {
        self.inner.lock().await.chunks.remove(hash);
        Ok(())
    }

    async fn list_chunks(&self) -> Result<Vec<String>, BackupError> {
        Ok(self.inner.lock().await.chunks.keys().cloned().collect())
    }
}

/// Filesystem-backed store. Chunks are compressed with zstd and sharded by
/// the first two hex chars of the SHA-256.
#[derive(Debug, Clone)]
pub struct FileBackupStore {
    root: PathBuf,
    /// Lock held across `write_manifest` + related writes so callers can
    /// serialize operations they care about.
    write_lock: Arc<Mutex<()>>,
}

impl FileBackupStore {
    /// Create a store rooted at `root`. The directory is created if missing.
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, BackupError> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("chunks"))
            .map_err(|e| BackupError::Storage(format!("mkdir chunks: {e}")))?;
        std::fs::create_dir_all(root.join("manifests"))
            .map_err(|e| BackupError::Storage(format!("mkdir manifests: {e}")))?;
        Ok(Self {
            root,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    fn chunk_path(&self, hash: &str) -> PathBuf {
        let prefix = if hash.len() >= 2 { &hash[..2] } else { "00" };
        self.root
            .join("chunks")
            .join(prefix)
            .join(format!("{hash}.zst"))
    }

    fn manifest_path(&self, name: &str) -> PathBuf {
        self.root.join("manifests").join(format!("{name}.json"))
    }
}

#[async_trait]
impl BackupStore for FileBackupStore {
    async fn put_chunk(&self, hash: &str, data: &[u8]) -> Result<(), BackupError> {
        let path = self.chunk_path(hash);
        let hash_owned = hash.to_string();
        let data_owned = data.to_vec();
        tokio::task::spawn_blocking(move || -> Result<(), BackupError> {
            // Verify the hash before writing.
            let actual = content_hash(&data_owned);
            if actual != hash_owned {
                return Err(BackupError::HashMismatch {
                    expected: hash_owned,
                    actual,
                });
            }
            if path.exists() {
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| BackupError::Storage(format!("mkdir: {e}")))?;
            }
            let compressed = zstd::encode_all(&data_owned[..], 3)
                .map_err(|e| BackupError::Storage(format!("zstd encode: {e}")))?;
            let tmp = path.with_extension("zst.tmp");
            std::fs::write(&tmp, &compressed)
                .map_err(|e| BackupError::Storage(format!("write tmp: {e}")))?;
            std::fs::rename(&tmp, &path)
                .map_err(|e| BackupError::Storage(format!("rename: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))??;
        Ok(())
    }

    async fn get_chunk(&self, hash: &str) -> Result<Vec<u8>, BackupError> {
        let path = self.chunk_path(hash);
        let hash_owned = hash.to_string();
        let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, BackupError> {
            let compressed = std::fs::read(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    BackupError::ChunkMissing(hash_owned.clone())
                } else {
                    BackupError::Storage(format!("read {}: {e}", path.display()))
                }
            })?;
            let data = zstd::decode_all(&compressed[..])
                .map_err(|e| BackupError::Storage(format!("zstd decode: {e}")))?;
            let actual = content_hash(&data);
            if actual != hash_owned {
                return Err(BackupError::HashMismatch {
                    expected: hash_owned,
                    actual,
                });
            }
            Ok(data)
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))??;
        Ok(data)
    }

    async fn has_chunk(&self, hash: &str) -> Result<bool, BackupError> {
        let path = self.chunk_path(hash);
        Ok(tokio::task::spawn_blocking(move || path.exists())
            .await
            .map_err(|e| BackupError::Storage(format!("join: {e}")))?)
    }

    async fn write_manifest(&self, name: &str, manifest: &Manifest) -> Result<(), BackupError> {
        let _guard = self.write_lock.lock().await;
        let path = self.manifest_path(name);
        let bytes =
            serde_json::to_vec_pretty(manifest).map_err(|e| BackupError::Serde(e.to_string()))?;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))?
        .map_err(|e| BackupError::Storage(format!("manifest write: {e}")))?;
        Ok(())
    }

    async fn read_manifest(&self, name: &str) -> Result<Manifest, BackupError> {
        let path = self.manifest_path(name);
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(&path))
            .await
            .map_err(|e| BackupError::Storage(format!("join: {e}")))?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    BackupError::SnapshotNotFound(e.to_string())
                } else {
                    BackupError::Storage(e.to_string())
                }
            })?;
        serde_json::from_slice(&bytes).map_err(|e| BackupError::Serde(e.to_string()))
    }

    async fn list_manifests(&self) -> Result<Vec<String>, BackupError> {
        let dir = self.root.join("manifests");
        tokio::task::spawn_blocking(move || -> Result<Vec<String>, BackupError> {
            let mut out = Vec::new();
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
                Err(e) => return Err(BackupError::Storage(format!("read_dir: {e}"))),
            };
            for entry in entries {
                let entry = entry.map_err(|e| BackupError::Storage(format!("entry: {e}")))?;
                if let Some(stem) = entry
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                {
                    out.push(stem);
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))?
    }

    async fn delete_manifest(&self, name: &str) -> Result<(), BackupError> {
        let _guard = self.write_lock.lock().await;
        let path = self.manifest_path(name);
        tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BackupError::Storage(format!("remove: {e}"))),
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))?
    }

    async fn delete_chunk(&self, hash: &str) -> Result<(), BackupError> {
        let path = self.chunk_path(hash);
        tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BackupError::Storage(format!("remove: {e}"))),
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))?
    }

    async fn list_chunks(&self) -> Result<Vec<String>, BackupError> {
        let dir = self.root.join("chunks");
        tokio::task::spawn_blocking(move || -> Result<Vec<String>, BackupError> {
            let mut out = Vec::new();
            let shards = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
                Err(e) => return Err(BackupError::Storage(format!("read_dir: {e}"))),
            };
            for shard in shards {
                let shard = shard.map_err(|e| BackupError::Storage(format!("entry: {e}")))?;
                let sp = shard.path();
                if !sp.is_dir() {
                    continue;
                }
                for entry in std::fs::read_dir(&sp)
                    .map_err(|e| BackupError::Storage(format!("shard read: {e}")))?
                {
                    let entry = entry.map_err(|e| BackupError::Storage(format!("entry: {e}")))?;
                    if let Some(stem) = entry
                        .path()
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                    {
                        out.push(stem);
                    }
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| BackupError::Storage(format!("join: {e}")))?
    }
}

/// Stream-restore helper that pulls chunks from any `BackupStore`.
pub async fn restore_manifest_to<W, S>(
    store: &S,
    manifest: &Manifest,
    writer: &mut W,
) -> Result<(), BackupError>
where
    W: tokio::io::AsyncWrite + Unpin,
    S: BackupStore + ?Sized,
{
    for hash in &manifest.chunk_ids {
        let data = store.get_chunk(hash).await?;
        // `get_chunk` already verifies hash on both impls, but double-check
        // defensively.
        let actual = content_hash(&data);
        if actual != *hash {
            return Err(BackupError::HashMismatch {
                expected: hash.clone(),
                actual,
            });
        }
        writer
            .write_all(&data)
            .await
            .map_err(|e| BackupError::Storage(format!("write: {e}")))?;
    }
    writer
        .flush()
        .await
        .map_err(|e| BackupError::Storage(format!("flush: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Chunk, Manifest, chunk_data};

    fn sample_manifest(chunks: &[Chunk]) -> Manifest {
        Manifest {
            id: "m1".into(),
            hostname: "host".into(),
            paths: vec!["/a".into()],
            created_at: chrono::Utc::now(),
            total_size: chunks.iter().map(|c| c.original_size as u64).sum(),
            chunk_ids: chunks.iter().map(|c| c.id.clone()).collect(),
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn file_store_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileBackupStore::new(tmp.path()).unwrap();

        let chunks = chunk_data(b"hello streaming world, please round trip", 8);
        for c in &chunks {
            store.put_chunk(&c.id, &c.data).await.unwrap();
            assert!(store.has_chunk(&c.id).await.unwrap());
        }

        let manifest = sample_manifest(&chunks);
        store.write_manifest("snap1", &manifest).await.unwrap();

        let loaded = store.read_manifest("snap1").await.unwrap();
        assert_eq!(loaded.chunk_ids, manifest.chunk_ids);

        let mut out: Vec<u8> = Vec::new();
        restore_manifest_to(&store, &loaded, &mut out)
            .await
            .unwrap();

        let expected: Vec<u8> = chunks.iter().flat_map(|c| c.data.clone()).collect();
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn file_store_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FileBackupStore::new(tmp.path()).unwrap();

        let chunks = chunk_data(b"detect corruption on read path", 8);
        let first = &chunks[0];
        store.put_chunk(&first.id, &first.data).await.unwrap();

        // Corrupt the stored file by rewriting it with garbage.
        let path = store.chunk_path(&first.id);
        std::fs::write(&path, b"not a valid zstd payload").unwrap();

        let err = store.get_chunk(&first.id).await.unwrap_err();
        // Could fail at zstd decode (Storage) or hash mismatch if the
        // payload happened to decode; either outcome rejects the fetch.
        match err {
            BackupError::HashMismatch { .. } | BackupError::Storage(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn gc_prunes_unreferenced_chunks() {
        let store = InMemoryBackupStore::new();
        let chunks_a = chunk_data(b"alpha only", 4);
        let chunks_b = chunk_data(b"beta only, different", 4);
        for c in chunks_a.iter().chain(chunks_b.iter()) {
            store.put_chunk(&c.id, &c.data).await.unwrap();
        }
        // Only reference chunks_a via a manifest.
        let manifest = sample_manifest(&chunks_a);
        store.write_manifest("a", &manifest).await.unwrap();

        let removed = gc_offline(&store).await.unwrap();
        assert!(removed > 0);
        for c in &chunks_a {
            assert!(store.has_chunk(&c.id).await.unwrap());
        }
    }
}
