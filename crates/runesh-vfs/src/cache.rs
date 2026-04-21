//! Local cache manager with LRU eviction for hydrated files.
//!
//! Files are cached locally after being fetched from the FileProvider.
//! When cache exceeds the configured maximum, least-recently-accessed files
//! are evicted (dehydrated) to free disk space.
//!
//! `put` and `evict` share a single [`tokio::sync::Mutex`]; a put that
//! pushes the cache over the limit triggers eviction **before** it is
//! acknowledged. This means `current_bytes <= max_bytes` always holds on
//! the boundary between calls.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::sync::Mutex;

use crate::error::VfsError;

/// A cached file entry.
#[derive(Debug, Clone)]
struct CacheEntry {
    path: String,
    size: u64,
    last_access: Instant,
}

/// Inner state protected by a single async mutex.
#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<String, CacheEntry>,
    current_bytes: u64,
}

/// LRU cache manager for locally-hydrated files.
pub struct CacheManager {
    cache_dir: PathBuf,
    max_bytes: u64,
    inner: Mutex<Inner>,
}

impl CacheManager {
    /// Create a new cache manager.
    pub async fn new(cache_dir: PathBuf, max_bytes: u64) -> Result<Self, VfsError> {
        tokio::fs::create_dir_all(&cache_dir).await?;

        let manager = Self {
            cache_dir,
            max_bytes,
            inner: Mutex::new(Inner::default()),
        };

        manager.scan_existing().await;

        Ok(manager)
    }

    /// Cache file content after hydration.
    ///
    /// Serializes against `evict` and other `put` calls so the cache size
    /// invariant is not violated by concurrent writers.
    pub async fn put(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let cache_file = self.cache_path(path)?;

        if let Some(parent) = cache_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&cache_file, data).await?;

        let size = data.len() as u64;
        let mut inner = self.inner.lock().await;

        // Replace any old entry with the new size.
        if let Some(old) = inner.entries.insert(
            path.to_string(),
            CacheEntry {
                path: path.to_string(),
                size,
                last_access: Instant::now(),
            },
        ) {
            inner.current_bytes = inner.current_bytes.saturating_sub(old.size);
        }
        inner.current_bytes = inner.current_bytes.saturating_add(size);

        // Evict before acknowledging the put.
        if inner.current_bytes > self.max_bytes {
            self.evict_lru_locked(&mut inner).await;
        }

        Ok(())
    }

    /// Get cached content. Returns None if not cached or evicted.
    pub async fn get(&self, path: &str) -> Option<Vec<u8>> {
        let cache_file = self.cache_path(path).ok()?;

        if !cache_file.exists() {
            return None;
        }

        // Update access time
        {
            let mut inner = self.inner.lock().await;
            if let Some(entry) = inner.entries.get_mut(path) {
                entry.last_access = Instant::now();
            }
        }

        tokio::fs::read(&cache_file).await.ok()
    }

    /// Check if a file is cached.
    pub async fn contains(&self, path: &str) -> bool {
        self.cache_path(path).map(|p| p.exists()).unwrap_or(false)
    }

    /// Evict a specific file from cache (dehydration).
    pub async fn evict(&self, path: &str) -> Result<(), VfsError> {
        let cache_file = self.cache_path(path)?;

        match tokio::fs::remove_file(&cache_file).await {
            Ok(()) => {
                let mut inner = self.inner.lock().await;
                let size = inner.entries.remove(path).map(|e| e.size).unwrap_or(0);
                inner.current_bytes = inner.current_bytes.saturating_sub(size);
                tracing::debug!(path = %path, size, "Cache: evicted file");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    /// Evict LRU entries while holding the inner lock. Called from `put`
    /// when the cache exceeds the configured maximum.
    async fn evict_lru_locked(&self, inner: &mut Inner) {
        let target = (self.max_bytes as f64 * 0.8) as u64;

        let mut sorted: Vec<_> = inner.entries.values().cloned().collect();
        sorted.sort_by_key(|e| e.last_access);

        for entry in sorted {
            if inner.current_bytes <= target {
                break;
            }
            let cache_file = match self.cache_path(&entry.path) {
                Ok(p) => p,
                Err(_) => {
                    // Drop a bad entry anyway.
                    inner.entries.remove(&entry.path);
                    continue;
                }
            };
            if tokio::fs::remove_file(&cache_file).await.is_ok() {
                inner.current_bytes = inner.current_bytes.saturating_sub(entry.size);
                inner.entries.remove(&entry.path);
                tracing::debug!(path = %entry.path, "Cache: LRU evicted");
            }
        }
    }

    /// Get current cache usage in bytes.
    pub async fn current_bytes(&self) -> u64 {
        self.inner.lock().await.current_bytes
    }

    /// Get maximum cache size in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Get the cache file path for a given VFS path.
    /// Validates that the result stays within the cache directory.
    fn cache_path(&self, path: &str) -> Result<PathBuf, VfsError> {
        if path.contains("..") || path.contains('\0') {
            return Err(VfsError::PathTraversal);
        }

        use std::path::Component;
        let mut normalized = self.cache_dir.clone();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(c) => normalized.push(c),
                Component::CurDir => {}
                _ => return Err(VfsError::PathTraversal),
            }
        }

        if !normalized.starts_with(&self.cache_dir) {
            return Err(VfsError::PathTraversal);
        }

        Ok(normalized)
    }

    /// Scan existing cache directory and populate entries map.
    async fn scan_existing(&self) {
        let mut inner = self.inner.lock().await;
        let mut total = 0u64;

        scan_dir_recursive(
            &self.cache_dir,
            &self.cache_dir,
            &mut inner.entries,
            &mut total,
        )
        .await;

        inner.current_bytes = total;
        tracing::debug!(
            files = inner.entries.len(),
            bytes = total,
            "Cache: scanned existing entries"
        );
    }
}

/// Recursively scan a directory for cache entries.
async fn scan_dir_recursive(
    base: &Path,
    dir: &Path,
    entries: &mut HashMap<String, CacheEntry>,
    total: &mut u64,
) {
    if let Ok(mut read_dir) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();

            if path.is_dir() {
                Box::pin(scan_dir_recursive(base, &path, entries, total)).await;
            } else if let Ok(metadata) = entry.metadata().await {
                let size = metadata.len();
                *total += size;

                if let Ok(rel) = path.strip_prefix(base) {
                    let key = rel.to_string_lossy().to_string();
                    entries.insert(
                        key.clone(),
                        CacheEntry {
                            path: key,
                            size,
                            last_access: Instant::now(),
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "runesh-vfs-cache-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4(),
        ))
    }

    #[tokio::test]
    async fn put_respects_max_bytes() {
        let dir = test_dir();
        let max = 1024u64;
        let cache = CacheManager::new(dir.clone(), max).await.unwrap();

        // Write 5x 512-byte entries sequentially. Expect eviction to
        // keep us at or below max.
        for i in 0..5 {
            let key = format!("file{i}.bin");
            cache.put(&key, &vec![0u8; 512]).await.unwrap();
            let now = cache.current_bytes().await;
            assert!(now <= max, "cache exceeded max_bytes: now={now} max={max}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
