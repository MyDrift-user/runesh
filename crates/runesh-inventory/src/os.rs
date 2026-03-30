//! Operating system information collection.

use sysinfo::System;

use crate::models::OsInfo;

/// Collect operating system information.
pub fn collect_os(_sys: &System) -> OsInfo {
    OsInfo {
        name: System::name().unwrap_or_else(|| "Unknown".into()),
        version: System::os_version().unwrap_or_else(|| "Unknown".into()),
        kernel_version: System::kernel_version().unwrap_or_else(|| "Unknown".into()),
        arch: std::env::consts::ARCH.to_string(),
        hostname: System::host_name().unwrap_or_else(|| "Unknown".into()),
        uptime_secs: System::uptime(),
        boot_time: System::boot_time(),
        distribution_id: System::distribution_id(),
    }
}
