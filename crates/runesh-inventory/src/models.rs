//! Data models for hardware and software inventory.
//!
//! All types derive Serialize/Deserialize for JSON transport and storage.

use serde::{Deserialize, Serialize};

/// Complete system inventory snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInventory {
    pub collected_at: chrono::DateTime<chrono::Utc>,
    pub hostname: String,
    pub inventory_id: String,
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
    pub network_interfaces: Vec<NetworkInterface>,
    pub gpus: Vec<GpuInfo>,
    pub bios: Option<BiosInfo>,
    pub battery: Option<BatteryInfo>,
    pub installed_software: Vec<InstalledSoftware>,
    pub processes: Vec<ProcessInfo>,
}

/// Operating system information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsInfo {
    pub name: String,
    pub version: String,
    pub kernel_version: String,
    pub arch: String,
    pub hostname: String,
    pub uptime_secs: u64,
    pub boot_time: u64,
    pub distribution_id: String,
}

/// CPU information (aggregate + per-core).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub brand: String,
    pub vendor: String,
    pub physical_cores: usize,
    pub logical_cores: usize,
    pub frequency_mhz: u64,
    pub usage_percent: f32,
    pub per_core_usage: Vec<CoreUsage>,
}

/// Per-core CPU usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUsage {
    pub core_id: usize,
    pub usage_percent: f32,
    pub frequency_mhz: u64,
}

/// Memory (RAM) information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub usage_percent: f32,
}

/// Disk/partition information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub disk_type: DiskType,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f32,
    pub is_removable: bool,
}

/// Disk type classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiskType {
    Ssd,
    Hdd,
    Removable,
    Unknown,
}

/// Network interface information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub mac_address: String,
    pub ip_addresses: Vec<IpAddress>,
    pub is_up: bool,
    pub bytes_received: u64,
    pub bytes_transmitted: u64,
    pub packets_received: u64,
    pub packets_transmitted: u64,
}

/// IP address with version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpAddress {
    pub address: String,
    pub prefix_len: u8,
    pub version: IpVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpVersion {
    V4,
    V6,
}

/// GPU information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub driver_version: String,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub temperature_celsius: Option<f32>,
}

/// BIOS/UEFI and motherboard information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiosInfo {
    pub bios_vendor: String,
    pub bios_version: String,
    pub bios_release_date: String,
    pub motherboard_manufacturer: String,
    pub motherboard_product: String,
    pub motherboard_serial: String,
    pub system_manufacturer: String,
    pub system_product: String,
    pub system_serial: String,
    pub system_uuid: String,
}

/// Battery information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub charge_percent: f32,
    pub is_charging: bool,
    pub is_plugged_in: bool,
    pub time_to_empty_mins: Option<u32>,
    pub time_to_full_mins: Option<u32>,
    pub health_percent: Option<f32>,
    pub cycle_count: Option<u32>,
    pub voltage_mv: Option<u32>,
    pub design_capacity_mwh: Option<u32>,
    pub full_charge_capacity_mwh: Option<u32>,
}

/// Installed software entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSoftware {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub install_date: String,
    pub install_location: String,
    pub size_bytes: Option<u64>,
}

/// Running process snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe_path: String,
    pub cmd: Vec<String>,
    pub status: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub user: Option<String>,
    pub start_time: u64,
    pub parent_pid: Option<u32>,
}

/// Delta report: changes between two inventory snapshots.
#[cfg(feature = "delta")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryDelta {
    pub from_id: String,
    pub to_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub changes: Vec<InventoryChange>,
}

#[cfg(feature = "delta")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryChange {
    pub category: String,
    pub field: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
}
