//! Mount point registry — manages VFS mount lifecycle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::cache::CacheManager;
use crate::config::VfsConfig;
use crate::error::VfsError;
use crate::platform::VfsMount;
use crate::provider::FileProvider;

/// Manages multiple VFS mount points.
pub struct MountRegistry {
    mounts: RwLock<HashMap<String, MountEntry>>,
}

struct MountEntry {
    config: VfsConfig,
    mount: VfsMount,
}

impl MountRegistry {
    pub fn new() -> Self {
        Self {
            mounts: RwLock::new(HashMap::new()),
        }
    }

    /// Mount a virtual filesystem.
    pub async fn mount(
        &self,
        id: &str,
        config: VfsConfig,
        provider: Arc<dyn FileProvider>,
        cache: Arc<CacheManager>,
    ) -> Result<PathBuf, VfsError> {
        let mounts = self.mounts.read().await;
        if mounts.contains_key(id) {
            return Err(VfsError::AlreadyMounted(id.into()));
        }
        drop(mounts);

        let mount_point = config.mount_point.clone();
        let mount = crate::platform::mount(config.clone(), provider, cache).await?;

        self.mounts
            .write()
            .await
            .insert(id.to_string(), MountEntry { config, mount });

        tracing::info!(id = %id, mount_point = %mount_point.display(), "VFS mounted");
        Ok(mount_point)
    }

    /// Unmount a virtual filesystem.
    pub async fn unmount(&self, id: &str) -> Result<(), VfsError> {
        let mut mounts = self.mounts.write().await;
        let entry = mounts
            .remove(id)
            .ok_or_else(|| VfsError::NotMounted(id.into()))?;

        tracing::info!(
            id = %id,
            mount_point = %entry.mount.mount_point().display(),
            "VFS unmounted"
        );
        // VfsMount::drop handles cleanup
        Ok(())
    }

    /// List all active mount points.
    pub async fn list_mounts(&self) -> Vec<MountInfo> {
        let mounts = self.mounts.read().await;
        mounts
            .iter()
            .map(|(id, entry)| MountInfo {
                id: id.clone(),
                mount_point: entry.mount.mount_point().to_path_buf(),
                display_name: entry.config.display_name.clone(),
                provider_id: entry.config.provider_id.clone(),
            })
            .collect()
    }

    /// Check if a mount ID is active.
    pub async fn is_mounted(&self, id: &str) -> bool {
        self.mounts.read().await.contains_key(id)
    }

    /// Unmount all filesystems.
    pub async fn unmount_all(&self) {
        let mut mounts = self.mounts.write().await;
        let count = mounts.len();
        mounts.clear();
        tracing::info!(count, "VFS: unmounted all filesystems");
    }
}

impl Default for MountRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about an active mount.
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub id: String,
    pub mount_point: PathBuf,
    pub display_name: String,
    pub provider_id: String,
}
