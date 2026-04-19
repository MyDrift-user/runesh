//! Axum REST handlers for inventory endpoints.
//!
//! Mount these on your router:
//! ```ignore
//! use runesh_inventory::handlers;
//! let app = Router::new()
//!     .route("/api/inventory", get(handlers::get_full_inventory))
//!     .route("/api/inventory/quick", get(handlers::get_quick_inventory))
//!     .route("/api/inventory/cpu", get(handlers::get_cpu))
//!     .route("/api/inventory/memory", get(handlers::get_memory))
//!     .route("/api/inventory/disks", get(handlers::get_disks))
//!     .route("/api/inventory/network", get(handlers::get_network))
//!     .route("/api/inventory/processes", get(handlers::get_processes))
//!     .route("/api/inventory/software", get(handlers::get_software));
//! ```

#[cfg(feature = "axum")]
mod axum_handlers {
    use axum::Json;

    use crate::collector::{CollectorConfig, collect_inventory, collect_quick_inventory};
    use crate::error::InventoryError;
    use crate::models::*;
    use crate::{cpu, disk, memory, network, process, software};

    /// GET /api/inventory — Full system inventory.
    pub async fn get_full_inventory() -> Result<Json<SystemInventory>, InventoryError> {
        let inventory =
            tokio::task::spawn_blocking(|| collect_inventory(&CollectorConfig::default()))
                .await
                .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))??;

        Ok(Json(inventory))
    }

    /// GET /api/inventory/quick — Lightweight inventory (no processes/software/BIOS).
    pub async fn get_quick_inventory() -> Result<Json<SystemInventory>, InventoryError> {
        let inventory = tokio::task::spawn_blocking(collect_quick_inventory)
            .await
            .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))??;

        Ok(Json(inventory))
    }

    /// GET /api/inventory/cpu — CPU info only.
    pub async fn get_cpu() -> Result<Json<CpuInfo>, InventoryError> {
        let info = tokio::task::spawn_blocking(|| {
            let mut sys = sysinfo::System::new();
            sys.refresh_cpu_all();
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            sys.refresh_cpu_all();
            cpu::collect_cpu(&sys)
        })
        .await
        .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }

    /// GET /api/inventory/memory — Memory info only.
    pub async fn get_memory() -> Result<Json<MemoryInfo>, InventoryError> {
        let info = tokio::task::spawn_blocking(|| {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            memory::collect_memory(&sys)
        })
        .await
        .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }

    /// GET /api/inventory/disks — Disk info only.
    pub async fn get_disks() -> Result<Json<Vec<DiskInfo>>, InventoryError> {
        let info = tokio::task::spawn_blocking(disk::collect_disks)
            .await
            .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }

    /// GET /api/inventory/network — Network interface info only.
    pub async fn get_network() -> Result<Json<Vec<NetworkInterface>>, InventoryError> {
        let info = tokio::task::spawn_blocking(network::collect_networks)
            .await
            .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }

    /// GET /api/inventory/processes — Running processes snapshot.
    pub async fn get_processes() -> Result<Json<Vec<ProcessInfo>>, InventoryError> {
        let info = tokio::task::spawn_blocking(|| {
            let mut sys = sysinfo::System::new_all();
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            process::collect_processes(&sys)
        })
        .await
        .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }

    /// GET /api/inventory/software — Installed software list.
    pub async fn get_software() -> Result<Json<Vec<InstalledSoftware>>, InventoryError> {
        let info = tokio::task::spawn_blocking(software::collect_software)
            .await
            .map_err(|e| InventoryError::Internal(format!("Task join error: {e}")))?;

        Ok(Json(info))
    }
}

#[cfg(feature = "axum")]
pub use axum_handlers::*;
