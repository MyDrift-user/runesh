//! Windows-specific inventory collection using WMI.

use std::collections::HashMap;

use wmi::{COMLibrary, WMIConnection};

use crate::models::{BatteryInfo, BiosInfo, GpuInfo, InstalledSoftware};

fn wmi_connect() -> Option<WMIConnection> {
    let com = COMLibrary::new().ok()?;
    WMIConnection::new(com).ok()
}

/// Query WMI and return results as Vec of HashMaps.
fn wmi_query(conn: &WMIConnection, query: &str) -> Vec<HashMap<String, serde_json::Value>> {
    conn.raw_query::<HashMap<String, serde_json::Value>>(query)
        .unwrap_or_default()
}

fn get_str(map: &HashMap<String, serde_json::Value>, key: &str) -> String {
    map.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn get_u64(map: &HashMap<String, serde_json::Value>, key: &str) -> u64 {
    map.get(key)
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

fn get_f32(map: &HashMap<String, serde_json::Value>, key: &str) -> f32 {
    map.get(key)
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0) as f32
}

/// Collect GPU info via WMI Win32_VideoController.
pub fn collect_gpus_wmi() -> Vec<GpuInfo> {
    let Some(conn) = wmi_connect() else {
        tracing::warn!("Failed to connect to WMI for GPU info");
        return Vec::new();
    };

    let results = wmi_query(
        &conn,
        "SELECT Name, AdapterCompatibility, DriverVersion, AdapterRAM FROM Win32_VideoController",
    );

    results
        .iter()
        .map(|gpu| GpuInfo {
            name: get_str(gpu, "Name"),
            vendor: get_str(gpu, "AdapterCompatibility"),
            driver_version: get_str(gpu, "DriverVersion"),
            memory_total_bytes: Some(get_u64(gpu, "AdapterRAM")),
            memory_used_bytes: None,
            temperature_celsius: None,
        })
        .collect()
}

/// Collect BIOS/motherboard info via WMI.
pub fn collect_bios_wmi() -> Option<BiosInfo> {
    let conn = wmi_connect()?;

    let bios_results = wmi_query(
        &conn,
        "SELECT Manufacturer, SMBIOSBIOSVersion, ReleaseDate FROM Win32_BIOS",
    );
    let board_results = wmi_query(
        &conn,
        "SELECT Manufacturer, Product, SerialNumber FROM Win32_BaseBoard",
    );
    let system_results = wmi_query(
        &conn,
        "SELECT Manufacturer, Model, SerialNumber, UUID FROM Win32_ComputerSystemProduct",
    );

    let bios = bios_results.first();
    let board = board_results.first();
    let system = system_results.first();

    Some(BiosInfo {
        bios_vendor: bios.map(|b| get_str(b, "Manufacturer")).unwrap_or_default(),
        bios_version: bios
            .map(|b| get_str(b, "SMBIOSBIOSVersion"))
            .unwrap_or_default(),
        bios_release_date: bios.map(|b| get_str(b, "ReleaseDate")).unwrap_or_default(),
        motherboard_manufacturer: board
            .map(|b| get_str(b, "Manufacturer"))
            .unwrap_or_default(),
        motherboard_product: board.map(|b| get_str(b, "Product")).unwrap_or_default(),
        motherboard_serial: board
            .map(|b| get_str(b, "SerialNumber"))
            .unwrap_or_default(),
        system_manufacturer: system
            .map(|s| get_str(s, "Manufacturer"))
            .unwrap_or_default(),
        system_product: system.map(|s| get_str(s, "Model")).unwrap_or_default(),
        system_serial: system
            .map(|s| get_str(s, "SerialNumber"))
            .unwrap_or_default(),
        system_uuid: system.map(|s| get_str(s, "UUID")).unwrap_or_default(),
    })
}

/// Collect battery info via WMI Win32_Battery.
pub fn collect_battery_wmi() -> Option<BatteryInfo> {
    let conn = wmi_connect()?;

    let results = wmi_query(
        &conn,
        "SELECT EstimatedChargeRemaining, BatteryStatus, EstimatedRunTime, DesignVoltage FROM Win32_Battery",
    );

    let bat = results.first()?;
    let charge = get_f32(bat, "EstimatedChargeRemaining");
    let status = get_u64(bat, "BatteryStatus");

    // BatteryStatus: 1=Discharging, 2=AC, 3=Full, 4=Low, 5=Critical, 6=Charging
    let is_charging = status == 6;
    let is_plugged_in = status == 2 || status == 3 || status == 6;

    let run_time = get_u64(bat, "EstimatedRunTime");
    let time_to_empty = if !is_charging && run_time > 0 && run_time < 71582788 {
        Some(run_time as u32)
    } else {
        None
    };

    Some(BatteryInfo {
        charge_percent: charge,
        is_charging,
        is_plugged_in,
        time_to_empty_mins: time_to_empty,
        time_to_full_mins: None,
        health_percent: None,
        cycle_count: None,
        voltage_mv: Some(get_u64(bat, "DesignVoltage") as u32),
        design_capacity_mwh: None,
        full_charge_capacity_mwh: None,
    })
}

/// Collect installed software via WMI Win32_Product (slow) and registry fallback.
pub fn collect_software_wmi() -> Vec<InstalledSoftware> {
    let Some(conn) = wmi_connect() else {
        return Vec::new();
    };

    // Win32Reg_AddRemovePrograms is faster than Win32_Product
    // Fall back to Win32_Product if needed
    let results = wmi_query(
        &conn,
        "SELECT Name, Version, Vendor, InstallDate, InstallLocation FROM Win32_Product",
    );

    results
        .iter()
        .filter(|sw| !get_str(sw, "Name").is_empty())
        .map(|sw| InstalledSoftware {
            name: get_str(sw, "Name"),
            version: get_str(sw, "Version"),
            publisher: get_str(sw, "Vendor"),
            install_date: get_str(sw, "InstallDate"),
            install_location: get_str(sw, "InstallLocation"),
            size_bytes: None,
        })
        .collect()
}
