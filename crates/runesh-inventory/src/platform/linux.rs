//! Linux-specific inventory collection via /proc, /sys, and system commands.

use std::fs;
use std::process::Command;

use crate::models::{BatteryInfo, BiosInfo, GpuInfo, InstalledSoftware};

/// Read a sysfs file, trimming whitespace. Returns empty string on failure.
fn read_sysfs(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Collect GPU info from /sys and lspci.
///
/// NVIDIA GPU entries under `/proc/driver/nvidia/gpus/<pci-bus>/information`
/// are matched against lspci output by PCI bus address so indices don't need
/// to coincide. Falls back to lspci-only data if no NVIDIA entry matches.
pub fn collect_gpus_linux() -> Vec<GpuInfo> {
    let mut gpus: Vec<(String, GpuInfo)> = Vec::new();

    if let Ok(output) = Command::new("lspci").args(["-vnnn"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut current: Option<(String, GpuInfo)> = None;

        for line in stdout.lines() {
            if line.contains("VGA compatible controller")
                || line.contains("3D controller")
                || line.contains("Display controller")
            {
                if let Some(g) = current.take() {
                    gpus.push(g);
                }
                let bus = line.split_whitespace().next().unwrap_or("").to_string();
                let name = line
                    .split(']')
                    .last()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| line.to_string());
                current = Some((
                    bus,
                    GpuInfo {
                        name,
                        vendor: String::new(),
                        driver_version: String::new(),
                        memory_total_bytes: None,
                        memory_used_bytes: None,
                        temperature_celsius: None,
                    },
                ));
            } else if let Some((_, gpu)) = current.as_mut() {
                let trimmed = line.trim();
                if trimmed.starts_with("Kernel driver in use:") {
                    gpu.driver_version = trimmed
                        .strip_prefix("Kernel driver in use:")
                        .unwrap_or("")
                        .trim()
                        .to_string();
                }
            }
        }

        if let Some(g) = current {
            gpus.push(g);
        }
    }

    // NVIDIA: match /proc/driver/nvidia/gpus/<bus> against lspci bus addresses.
    if let Ok(entries) = fs::read_dir("/proc/driver/nvidia/gpus") {
        for entry in entries.flatten() {
            let Some(bus_name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            let info_path = entry.path().join("information");
            let Ok(content) = fs::read_to_string(&info_path) else {
                continue;
            };

            let mut nvidia_model = None;
            for line in content.lines() {
                if let Some(m) = line.strip_prefix("Model:") {
                    nvidia_model = Some(m.trim().to_string());
                }
            }
            if let Some(model) = nvidia_model {
                if let Some((_, gpu)) = gpus
                    .iter_mut()
                    .find(|(bus, _)| pci_bus_matches(bus, &bus_name))
                {
                    gpu.name = model;
                    gpu.vendor = "NVIDIA".into();
                }
            }
        }
    }

    gpus.into_iter().map(|(_, g)| g).collect()
}

/// Compare a lspci bus ID (`01:00.0`) against an NVIDIA proc bus ID
/// (`0000:01:00.0` or `01:00.0`). Accepts with or without the domain prefix.
fn pci_bus_matches(lspci_bus: &str, nvidia_bus: &str) -> bool {
    let strip_domain = |s: &str| -> String {
        s.split_once(':')
            .and_then(|(a, rest)| {
                if a.chars().all(|c| c.is_ascii_hexdigit()) && a.len() == 4 {
                    Some(rest.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| s.to_string())
    };
    let a = strip_domain(lspci_bus).to_lowercase();
    let b = strip_domain(nvidia_bus).to_lowercase();
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_bus_matches_with_and_without_domain() {
        assert!(pci_bus_matches("01:00.0", "0000:01:00.0"));
        assert!(pci_bus_matches("0000:01:00.0", "01:00.0"));
        assert!(pci_bus_matches("0000:01:00.0", "0000:01:00.0"));
        assert!(!pci_bus_matches("01:00.0", "02:00.0"));
    }
}

/// Collect BIOS/motherboard info from DMI sysfs.
pub fn collect_bios_linux() -> Option<BiosInfo> {
    let dmi = "/sys/class/dmi/id";

    // Check if DMI directory exists
    if !std::path::Path::new(dmi).exists() {
        return None;
    }

    Some(BiosInfo {
        bios_vendor: read_sysfs(&format!("{dmi}/bios_vendor")),
        bios_version: read_sysfs(&format!("{dmi}/bios_version")),
        bios_release_date: read_sysfs(&format!("{dmi}/bios_date")),
        motherboard_manufacturer: read_sysfs(&format!("{dmi}/board_vendor")),
        motherboard_product: read_sysfs(&format!("{dmi}/board_name")),
        motherboard_serial: read_sysfs(&format!("{dmi}/board_serial")),
        system_manufacturer: read_sysfs(&format!("{dmi}/sys_vendor")),
        system_product: read_sysfs(&format!("{dmi}/product_name")),
        system_serial: read_sysfs(&format!("{dmi}/product_serial")),
        system_uuid: read_sysfs(&format!("{dmi}/product_uuid")),
    })
}

/// Collect battery info from /sys/class/power_supply.
pub fn collect_battery_linux() -> Option<BatteryInfo> {
    let ps_dir = "/sys/class/power_supply";
    let entries = fs::read_dir(ps_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let supply_type = read_sysfs(&format!("{}/type", path.display()));

        if supply_type != "Battery" {
            continue;
        }

        let capacity = read_sysfs(&format!("{}/capacity", path.display()))
            .parse::<f32>()
            .unwrap_or(0.0);

        let status = read_sysfs(&format!("{}/status", path.display()));
        let is_charging = status == "Charging";
        let is_plugged_in = status != "Discharging";

        let energy_full = read_sysfs(&format!("{}/energy_full", path.display()))
            .parse::<u32>()
            .ok();
        let energy_full_design = read_sysfs(&format!("{}/energy_full_design", path.display()))
            .parse::<u32>()
            .ok();
        let voltage = read_sysfs(&format!("{}/voltage_now", path.display()))
            .parse::<u32>()
            .ok()
            .map(|v| v / 1000); // µV → mV

        let cycle_count = read_sysfs(&format!("{}/cycle_count", path.display()))
            .parse::<u32>()
            .ok();

        let health = match (energy_full, energy_full_design) {
            (Some(full), Some(design)) if design > 0 => Some((full as f32 / design as f32) * 100.0),
            _ => None,
        };

        return Some(BatteryInfo {
            charge_percent: capacity,
            is_charging,
            is_plugged_in,
            time_to_empty_mins: None,
            time_to_full_mins: None,
            health_percent: health,
            cycle_count,
            voltage_mv: voltage,
            design_capacity_mwh: energy_full_design.map(|v| v / 1000), // µWh → mWh
            full_charge_capacity_mwh: energy_full.map(|v| v / 1000),
        });
    }

    None
}

/// Collect installed software via dpkg or rpm.
pub fn collect_software_linux() -> Vec<InstalledSoftware> {
    // Try dpkg first (Debian/Ubuntu)
    if let Ok(output) = Command::new("dpkg-query")
        .args([
            "-W",
            "-f",
            "${Package}\t${Version}\t${Maintainer}\t${Installed-Size}\n",
        ])
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 2 {
                        Some(InstalledSoftware {
                            name: parts[0].to_string(),
                            version: parts[1].to_string(),
                            publisher: parts.get(2).unwrap_or(&"").to_string(),
                            install_date: String::new(),
                            install_location: String::new(),
                            size_bytes: parts
                                .get(3)
                                .and_then(|s| s.parse::<u64>().ok())
                                .map(|kb| kb * 1024),
                        })
                    } else {
                        None
                    }
                })
                .collect();
        }
    }

    // Try rpm (RHEL/Fedora/SUSE)
    if let Ok(output) = Command::new("rpm")
        .args([
            "-qa",
            "--queryformat",
            "%{NAME}\t%{VERSION}-%{RELEASE}\t%{VENDOR}\t%{INSTALLTIME}\t%{SIZE}\n",
        ])
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 2 {
                        Some(InstalledSoftware {
                            name: parts[0].to_string(),
                            version: parts[1].to_string(),
                            publisher: parts.get(2).unwrap_or(&"").to_string(),
                            install_date: parts.get(3).unwrap_or(&"").to_string(),
                            install_location: String::new(),
                            size_bytes: parts.get(4).and_then(|s| s.parse::<u64>().ok()),
                        })
                    } else {
                        None
                    }
                })
                .collect();
        }
    }

    Vec::new()
}
