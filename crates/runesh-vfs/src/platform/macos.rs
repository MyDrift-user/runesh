//! macOS FUSE implementation (via FUSE-T / macFUSE).
//!
//! Uses the same `fuser` crate as Linux. Requires FUSE-T or macFUSE
//! to be installed on the system.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::CacheManager;
use crate::config::VfsConfig;
use crate::error::VfsError;
use crate::provider::FileProvider;

/// macOS FUSE mount — delegates to the same FUSE implementation as Linux.
/// Requires FUSE-T (recommended) or macFUSE to be installed.
pub struct MacOsFuseMount {
    mount_point: PathBuf,
    // The FUSE session is held by the inner Linux implementation
    _inner: super::linux::LinuxFuseMount,
}

impl MacOsFuseMount {
    pub async fn mount(
        config: VfsConfig,
        provider: Arc<dyn FileProvider>,
        cache: Arc<CacheManager>,
    ) -> Result<Self, VfsError> {
        // macOS uses the same fuser API as Linux
        let mount_point = config.mount_point.clone();
        let inner = super::linux::LinuxFuseMount::mount(config, provider, cache).await?;

        Ok(Self {
            mount_point,
            _inner: inner,
        })
    }
}

impl super::VfsMountInner for MacOsFuseMount {
    fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}
