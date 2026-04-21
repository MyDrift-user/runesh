//! Windows Cloud Filter API (cfapi) implementation.
//!
//! Shows files as cloud placeholders in Windows Explorer with on-demand hydration.
//! Uses the same API as OneDrive's "Files On-Demand" feature.
//!
//! Requires Windows 10 version 1709 (Fall Creators Update) or later.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::CacheManager;
use crate::config::VfsConfig;
use crate::error::VfsError;
use crate::provider::FileProvider;

/// Windows Cloud Filter mount.
///
/// On creation, registers the mount point as a Cloud Sync Root with Windows.
/// Files appear in Explorer with cloud status icons (blue cloud = dehydrated,
/// green check = hydrated). Content is fetched on-demand via callbacks.
pub struct WindowsCloudFilter {
    mount_point: PathBuf,
    _provider: Arc<dyn FileProvider>,
    _cache: Arc<CacheManager>,
    _config: VfsConfig,
    is_connected: bool,
}

impl WindowsCloudFilter {
    /// Mount a virtual filesystem using the Cloud Filter API.
    pub async fn mount(
        config: VfsConfig,
        provider: Arc<dyn FileProvider>,
        cache: Arc<CacheManager>,
    ) -> Result<Self, VfsError> {
        let mount_point = config.mount_point.clone();

        // Ensure mount directory exists
        tokio::fs::create_dir_all(&mount_point).await?;

        // Register sync root in a blocking task
        let mount_point_clone = mount_point.clone();
        let config_clone = config.clone();

        let is_connected = tokio::task::spawn_blocking(move || {
            register_sync_root(&config_clone, &mount_point_clone)
        })
        .await
        .map_err(|e| VfsError::Platform(format!("Task join error: {e}")))??;

        tracing::info!(
            mount_point = %mount_point.display(),
            display_name = %config.display_name,
            "Windows Cloud Filter: registered sync root"
        );

        Ok(Self {
            mount_point,
            _provider: provider,
            _cache: cache,
            _config: config,
            is_connected,
        })
    }
}

impl super::VfsMountInner for WindowsCloudFilter {
    fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}

impl Drop for WindowsCloudFilter {
    fn drop(&mut self) {
        if self.is_connected
            && let Err(e) = unregister_sync_root(&self.mount_point)
        {
            tracing::error!(error = %e, "Failed to unregister sync root");
        }
        tracing::info!(
            mount_point = %self.mount_point.display(),
            "Windows Cloud Filter: unmounted"
        );
    }
}

/// Register a Cloud Filter sync root using cfapi.
fn register_sync_root(config: &VfsConfig, mount_point: &Path) -> Result<bool, VfsError> {
    use windows::Win32::Storage::CloudFilters::*;
    use windows::core::*;

    let path_wide: Vec<u16> = mount_point
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let display_wide: Vec<u16> = config
        .display_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let version_wide: Vec<u16> = config
        .provider_version
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // Build registration info
        let registration = CF_SYNC_REGISTRATION {
            StructSize: std::mem::size_of::<CF_SYNC_REGISTRATION>() as u32,
            ProviderName: PCWSTR(display_wide.as_ptr()),
            ProviderVersion: PCWSTR(version_wide.as_ptr()),
            ..Default::default()
        };

        let policies = CF_SYNC_POLICIES {
            StructSize: std::mem::size_of::<CF_SYNC_POLICIES>() as u32,
            ..Default::default()
        };

        // Register the sync root
        CfRegisterSyncRoot(
            PCWSTR(path_wide.as_ptr()),
            &registration,
            &policies,
            CF_REGISTER_FLAG_NONE,
        )
        .map_err(|e| VfsError::Platform(format!("CfRegisterSyncRoot failed: {e}")))?;

        tracing::debug!("Cloud Filter: sync root registered");

        // TODO: CfConnectSyncRoot with full callback table
        // The callback table requires careful FFI:
        // - CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS → list directory contents
        // - CF_CALLBACK_TYPE_FETCH_DATA → provide file content on open
        // - CF_CALLBACK_TYPE_CANCEL_FETCH_DATA → handle cancellation
        //
        // Each callback receives CF_CALLBACK_INFO with file identity,
        // and must call CfExecute to transfer data back to the OS.
        //
        // For now, the sync root is registered (shows in Explorer)
        // but callbacks are not yet connected.

        Ok(true)
    }
}

/// Unregister a Cloud Filter sync root.
fn unregister_sync_root(mount_point: &Path) -> Result<(), VfsError> {
    use windows::Win32::Storage::CloudFilters::*;
    use windows::core::*;

    let path_wide: Vec<u16> = mount_point
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        CfUnregisterSyncRoot(PCWSTR(path_wide.as_ptr()))
            .map_err(|e| VfsError::Platform(format!("CfUnregisterSyncRoot failed: {e}")))?;
    }

    Ok(())
}
