//! Disk and partition information collection.

use sysinfo::Disks;

use crate::models::{DiskInfo, DiskType};

/// Collect disk information from the system.
pub fn collect_disks() -> Vec<DiskInfo> {
    let disks = Disks::new_with_refreshed_list();

    disks
        .iter()
        .map(|disk| {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            let usage_percent = if total > 0 {
                (used as f32 / total as f32) * 100.0
            } else {
                0.0
            };

            let disk_type = match disk.kind() {
                sysinfo::DiskKind::SSD => DiskType::Ssd,
                sysinfo::DiskKind::HDD => DiskType::Hdd,
                _ => {
                    if disk.is_removable() {
                        DiskType::Removable
                    } else {
                        DiskType::Unknown
                    }
                }
            };

            DiskInfo {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                file_system: disk.file_system().to_string_lossy().to_string(),
                disk_type,
                total_bytes: total,
                available_bytes: available,
                used_bytes: used,
                usage_percent,
                is_removable: disk.is_removable(),
            }
        })
        .collect()
}
