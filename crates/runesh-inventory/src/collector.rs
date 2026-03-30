//! Main inventory collector that aggregates all subsystem collectors.

use sysinfo::System;

use crate::error::InventoryError;
use crate::models::SystemInventory;
use crate::{battery, bios, cpu, disk, gpu, memory, network, os, process, software};

/// Configuration for what to collect in the inventory.
#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub collect_processes: bool,
    pub collect_software: bool,
    pub collect_bios: bool,
    pub collect_battery: bool,
    pub collect_gpu: bool,
    /// Refresh CPU usage (requires a brief sleep for accurate readings).
    pub refresh_cpu: bool,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            collect_processes: true,
            collect_software: true,
            collect_bios: true,
            collect_battery: true,
            collect_gpu: true,
            refresh_cpu: true,
        }
    }
}

/// Collect a full system inventory snapshot.
///
/// This is a blocking operation that queries multiple system APIs.
/// For async contexts, wrap in `tokio::task::spawn_blocking`.
pub fn collect_inventory(config: &CollectorConfig) -> Result<SystemInventory, InventoryError> {
    let mut sys = System::new_all();

    if config.refresh_cpu {
        // First refresh gets baseline, second gets actual usage
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_all();
    }

    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
    let inventory_id = uuid::Uuid::new_v4().to_string();

    let os_info = os::collect_os(&sys);
    let cpu_info = cpu::collect_cpu(&sys);
    let memory_info = memory::collect_memory(&sys);
    let disks = disk::collect_disks();
    let networks = network::collect_networks();

    let gpus = if config.collect_gpu {
        gpu::collect_gpus()
    } else {
        Vec::new()
    };

    let bios_info = if config.collect_bios {
        bios::collect_bios()
    } else {
        None
    };

    let battery_info = if config.collect_battery {
        battery::collect_battery()
    } else {
        None
    };

    let installed_software = if config.collect_software {
        software::collect_software()
    } else {
        Vec::new()
    };

    let processes = if config.collect_processes {
        process::collect_processes(&sys)
    } else {
        Vec::new()
    };

    Ok(SystemInventory {
        collected_at: chrono::Utc::now(),
        hostname,
        inventory_id,
        os: os_info,
        cpu: cpu_info,
        memory: memory_info,
        disks,
        network_interfaces: networks,
        gpus,
        bios: bios_info,
        battery: battery_info,
        installed_software,
        processes,
    })
}

/// Collect a lightweight inventory (no processes, no software, no BIOS).
/// Faster for periodic monitoring.
pub fn collect_quick_inventory() -> Result<SystemInventory, InventoryError> {
    collect_inventory(&CollectorConfig {
        collect_processes: false,
        collect_software: false,
        collect_bios: false,
        collect_battery: true,
        collect_gpu: false,
        refresh_cpu: true,
    })
}
