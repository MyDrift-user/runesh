//! Platform-specific VFS mount implementations.
//!
//! - Windows: Cloud Filter API (cfapi) — placeholder files with on-demand hydration
//! - Linux: FUSE via `fuser` crate
//! - macOS: FUSE-T via `fuser` crate

use std::sync::Arc;

use crate::cache::CacheManager;
use crate::config::VfsConfig;
use crate::error::VfsError;
use crate::provider::FileProvider;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

/// Mount a virtual filesystem using the platform-native API.
///
/// On Windows, this uses the Cloud Filter API to show placeholder files
/// with cloud status icons in Explorer. On Linux/macOS, this uses FUSE.
pub async fn mount(
    config: VfsConfig,
    provider: Arc<dyn FileProvider>,
    cache: Arc<CacheManager>,
) -> Result<VfsMount, VfsError> {
    #[cfg(target_os = "windows")]
    {
        let mount = windows::WindowsCloudFilter::mount(config, provider, cache).await?;
        Ok(VfsMount {
            _inner: Box::new(mount),
        })
    }

    #[cfg(target_os = "linux")]
    {
        let mount = linux::LinuxFuseMount::mount(config, provider, cache).await?;
        Ok(VfsMount {
            _inner: Box::new(mount),
        })
    }

    #[cfg(target_os = "macos")]
    {
        let mount = macos::MacOsFuseMount::mount(config, provider, cache).await?;
        Ok(VfsMount {
            _inner: Box::new(mount),
        })
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = (config, provider, cache);
        Err(VfsError::Platform("Unsupported platform".into()))
    }
}

/// A mounted virtual filesystem handle.
/// The filesystem is unmounted when this is dropped.
pub struct VfsMount {
    _inner: Box<dyn VfsMountInner>,
}

impl VfsMount {
    /// Get the mount point path.
    pub fn mount_point(&self) -> &std::path::Path {
        self._inner.mount_point()
    }
}

/// Platform-specific mount implementation trait.
trait VfsMountInner: Send + Sync {
    fn mount_point(&self) -> &std::path::Path;
}
