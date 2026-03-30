//! BIOS/UEFI and motherboard information collection.
//!
//! Platform-specific: WMI on Windows, DMI on Linux, system_profiler on macOS.

use crate::models::BiosInfo;

/// Collect BIOS and motherboard information.
pub fn collect_bios() -> Option<BiosInfo> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::windows::collect_bios_wmi()
    }

    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::collect_bios_linux()
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::collect_bios_macos()
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
