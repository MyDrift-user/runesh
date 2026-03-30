//! macOS-specific inventory collection via system_profiler and IOKit.

use std::process::Command;

use crate::models::{BatteryInfo, BiosInfo, GpuInfo, InstalledSoftware};

/// Run system_profiler and parse JSON output for a given data type.
fn system_profiler_json(data_type: &str) -> Option<serde_json::Value> {
    let output = Command::new("system_profiler")
        .args([data_type, "-json"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    serde_json::from_slice(&output.stdout).ok()
}

/// Collect GPU info via system_profiler SPDisplaysDataType.
pub fn collect_gpus_macos() -> Vec<GpuInfo> {
    let Some(data) = system_profiler_json("SPDisplaysDataType") else {
        return Vec::new();
    };

    let displays = data.get("SPDisplaysDataType").and_then(|v| v.as_array());

    displays
        .map(|arr| {
            arr.iter()
                .map(|gpu| {
                    let vram = gpu
                        .get("sppci_vram")
                        .or_else(|| gpu.get("spdisplays_vram"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| {
                            let s = s.replace(" MB", "").replace(" GB", "000");
                            s.trim().parse::<u64>().ok().map(|mb| mb * 1024 * 1024)
                        });

                    GpuInfo {
                        name: gpu
                            .get("sppci_model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        vendor: gpu
                            .get("sppci_vendor")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        driver_version: String::new(),
                        memory_total_bytes: vram,
                        memory_used_bytes: None,
                        temperature_celsius: None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Collect BIOS/system info via system_profiler SPHardwareDataType.
pub fn collect_bios_macos() -> Option<BiosInfo> {
    let data = system_profiler_json("SPHardwareDataType")?;
    let hw = data
        .get("SPHardwareDataType")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())?;

    Some(BiosInfo {
        bios_vendor: "Apple".to_string(),
        bios_version: hw
            .get("boot_rom_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        bios_release_date: String::new(),
        motherboard_manufacturer: "Apple".to_string(),
        motherboard_product: hw
            .get("machine_model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        motherboard_serial: String::new(),
        system_manufacturer: "Apple".to_string(),
        system_product: hw
            .get("machine_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        system_serial: hw
            .get("serial_number")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        system_uuid: hw
            .get("platform_UUID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Collect battery info via system_profiler SPPowerDataType.
pub fn collect_battery_macos() -> Option<BatteryInfo> {
    let data = system_profiler_json("SPPowerDataType")?;
    let power = data
        .get("SPPowerDataType")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())?;

    // Battery info is nested under sppower_battery_information
    let bat_info = power.get("sppower_battery_information")?;
    let charge_info = power.get("sppower_battery_charge_info")?;

    let current_capacity = charge_info
        .get("sppower_battery_current_capacity")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let max_capacity = charge_info
        .get("sppower_battery_max_capacity")
        .and_then(|v| v.as_u64())
        .unwrap_or(100);

    let charge_percent = if max_capacity > 0 {
        (current_capacity as f32 / max_capacity as f32) * 100.0
    } else {
        0.0
    };

    let is_charging = charge_info
        .get("sppower_battery_is_charging")
        .and_then(|v| v.as_str())
        .map(|s| s == "TRUE")
        .unwrap_or(false);

    let cycle_count = bat_info
        .get("sppower_battery_cycle_count")
        .and_then(|v| v.as_u64())
        .map(|c| c as u32);

    let health = bat_info
        .get("sppower_battery_health")
        .and_then(|v| v.as_str())
        .and_then(|s| if s == "Good" { Some(100.0) } else { None });

    Some(BatteryInfo {
        charge_percent,
        is_charging,
        is_plugged_in: power
            .get("sppower_battery_charger_connected")
            .and_then(|v| v.as_str())
            .map(|s| s == "TRUE")
            .unwrap_or(false),
        time_to_empty_mins: None,
        time_to_full_mins: None,
        health_percent: health,
        cycle_count,
        voltage_mv: bat_info
            .get("sppower_battery_voltage")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32),
        design_capacity_mwh: None,
        full_charge_capacity_mwh: None,
    })
}

/// Collect installed software via system_profiler SPApplicationsDataType.
pub fn collect_software_macos() -> Vec<InstalledSoftware> {
    let Some(data) = system_profiler_json("SPApplicationsDataType") else {
        return Vec::new();
    };

    let apps = data
        .get("SPApplicationsDataType")
        .and_then(|v| v.as_array());

    apps.map(|arr| {
        arr.iter()
            .map(|app| InstalledSoftware {
                name: app
                    .get("_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                version: app
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                publisher: app
                    .get("obtained_from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                install_date: app
                    .get("lastModified")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                install_location: app
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                size_bytes: None,
            })
            .collect()
    })
    .unwrap_or_default()
}
