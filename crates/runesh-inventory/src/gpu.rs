//! GPU information collection.
//!
//! Uses platform-specific APIs for detailed GPU info.
//! Falls back to basic detection on unsupported platforms.

use crate::models::GpuInfo;

/// Collect GPU information from the system.
pub fn collect_gpus() -> Vec<GpuInfo> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::windows::collect_gpus_wmi()
    }

    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::collect_gpus_linux()
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::collect_gpus_macos()
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}
