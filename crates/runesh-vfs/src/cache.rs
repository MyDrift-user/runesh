//! Local cache manager with LRU eviction for hydrated files.
//!
//! Files are cached locally after being fetched from the FileProvider.
//! When cache exceeds the configured maximum, least-recently-accessed files
//! are evicted (dehydrated) to free disk space.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::RwLock;

use crate::error::VfsError;

/// A cached file entry.
#[derive(Debug, Clone)]
struct CacheEntry {
    path: String,
    size: u64,
    last_access: Instant,
}

/// LRU cache manager for locally-hydrated files.
pub struct CacheManager {
    cache_dir: PathBuf,
    max_bytes: u64,
    current_bytes: AtomicU64,
    entries: RwLock<HashMap<String, CacheEntry>>,
}

impl CacheManager {
    /// Create a new cache manager.
    pub async fn new(cache_dir: PathBuf, max_bytes: u64) -> Result<Self, VfsError> {
        tokio::fs::create_dir_all(&cache_dir).await?;

        let manager = Self {
            cache_dir,
            max_bytes,
            current_bytes: AtomicU64::new(0),
            entries: RwLock::new(HashMap::new()),
        };

        // Scan existing cache
        manager.scan_existing().await;

        Ok(manager)
    }

    /// Cache file content after hydration.
    pub async fn put(&self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        let cache_file = self.cache_path(path);

        if let Some(parent) = cache_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&cache_file, data).await?;

        let size = data.len() as u64;
        self.current_bytes.fetch_add(size, Ordering::Relaxed);

        self.entries.write().await.insert(
            path.to_string(),
            CacheEntry {
                path: path.to_string(),
                size,
                last_access: Instant::now(),
            },
        );

        // Evict if over limit
        let current = self.current_bytes.load(Ordering::Relaxed);
        if current > self.max_bytes {
            self.evict_lru().await;
        }

        Ok(())
    }

    /// Get cached content. Returns None if not cached or evicted.
    pub async fn get(&self, path: &str) -> Option<Vec<u8>> {
        let cache_file = self.cache_path(path);

        if !cache_file.exists() {
            return None;
        }

        // Update access time
        if let Some(entry) = self.entries.write().await.get_mut(path) {
            entry.last_access = Instant::now();
        }

        tokio::fs::read(&cache_file).await.ok()
    }

    /// Check if a file is cached.
    pub async fn contains(&self, path: &str) -> bool {
        self.cache_path(path).exists()
    }

    /// Evict a specific file from cache (dehydration).
    pub async fn evict(&self, path: &str) -> Result<(), VfsError> {
        let cache_file = self.cache_path(path);

        if cache_file.exists() {
            let size = tokio::fs::metadata(&cache_file)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            tokio::fs::remove_file(&cache_file).await?;
            self.current_bytes.fetch_sub(size, Ordering::Relaxed);
            self.entries.write().await.remove(path);

            tracing::debug!(path = %path, size, "Cache: evicted file");
        }

        Ok(())
    }

    /// Evict least-recently-used entries until under the max size.
    async fn evict_lru(&self) {
        let target = (self.max_bytes as f64 * 0.8) as u64; // Evict to 80% of max

        let mut entries = self.entries.write().await;
        let mut sorted: Vec<_> = entries.values().cloned().collect();
        sorted.sort_by_key(|e| e.last_access);

        let mut current = self.current_bytes.load(Ordering::Relaxed);

        for entry in &sorted {
            if current <= target {
                break;
            }

            let cache_file = self.cache_path(&entry.path);
            if tokio::fs::remove_file(&cache_file).await.is_ok() {
                current -= entry.size;
                entries.remove(&entry.path);
                tracing::debug!(path = %entry.path, "Cache: LRU evicted");
            }
        }

        self.current_bytes.store(current, Ordering::Relaxed);
    }

    /// Get current cache usage in bytes.
    pub fn current_bytes(&self) -> u64 {
        self.current_bytes.load(Ordering::Relaxed)
    }

    /// Get maximum cache size in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Get the cache file path for a given VFS path.
    fn cache_path(&self, path: &str) -> PathBuf {
        self.cache_dir.join(path)
    }

    /// Scan existing cache directory and populate entries map.
    async fn scan_existing(&self) {
        let mut total = 0u64;
        let mut entries = self.entries.write().await;

        scan_dir_recursive(&self.cache_dir, &self.cache_dir, &mut entries, &mut total).await;

        self.current_bytes.store(total, Ordering::Relaxed);
        tracing::debug!(
            files = entries.len(),
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
