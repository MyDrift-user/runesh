//! Installed software enumeration.
//!
//! Platform-specific: Registry on Windows, dpkg/rpm on Linux, system_profiler on macOS.

use crate::models::InstalledSoftware;

/// Collect list of installed software.
pub fn collect_software() -> Vec<InstalledSoftware> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::windows::collect_software_wmi()
    }

    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::collect_software_linux()
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::collect_software_macos()
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}
