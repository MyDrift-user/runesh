//! Battery information collection.
//!
//! Platform-specific battery status for laptops and portable devices.

use crate::models::BatteryInfo;

/// Collect battery information if available.
pub fn collect_battery() -> Option<BatteryInfo> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::windows::collect_battery_wmi()
    }

    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::collect_battery_linux()
    }

    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::collect_battery_macos()
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
