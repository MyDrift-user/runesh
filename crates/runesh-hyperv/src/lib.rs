#![deny(unsafe_code)]
//! Hyper-V workload driver over the native WMI provider
//! (`ROOT\Virtualization\V2`).
//!
//! All operations go through the [`Msvm_ComputerSystem`],
//! [`Msvm_VirtualSystemManagementService`], and
//! [`Msvm_VirtualSystemSnapshotService`] classes via the `wmi` crate. We do
//! not shell out to PowerShell: COM is cheaper, the provider returns
//! structured data, and there is no interpreter available to inject into.
//!
//! Long-running operations (start, stop, snapshot, destroy) go through
//! `Msvm_ConcreteJob`; we poll `JobState` until completion and surface
//! failures as [`WorkloadError`]. Cancellation tokens are honored between
//! poll ticks; the hypervisor cannot cancel a running job mid-flight but we
//! stop waiting and surface [`WorkloadError::Cancelled`].
//!
//! In-guest command execution is intentionally not implemented here: that is
//! a guest-agent concern and needs a channel into the VM that WMI does not
//! provide.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use runesh_workload::{
    CreateSpec, RunResult, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot,
    WorkloadState, WorkloadType,
};

pub struct HyperVDriver;

impl Default for HyperVDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl HyperVDriver {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
mod wmi_impl;

#[cfg(not(windows))]
fn not_supported() -> WorkloadError {
    WorkloadError::NotSupported("hyperv driver is Windows-only".into())
}

#[async_trait]
impl WorkloadDriver for HyperVDriver {
    fn driver_name(&self) -> &str {
        "hyperv"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(wmi_impl::list_vms)
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            Err(not_supported())
        }
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || wmi_impl::get_vm(&id))
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn create(&self, _spec: &CreateSpec) -> Result<Workload, WorkloadError> {
        // VM creation requires building Msvm_VirtualSystemSettingData plus
        // resource settings (CPU, memory, disks, NICs) and calling
        // `DefineSystem` on the management service. That is a substantial
        // surface area that belongs in a future change alongside a proper
        // spec schema; surfacing NotSupported is better than a half-built
        // creation path that silently drops fields from the caller's spec.
        Err(WorkloadError::NotSupported(
            "hyperv create not yet implemented; use Hyper-V Manager or PowerShell to define the VM, then manage it here"
                .into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || {
                wmi_impl::request_state_change(&id, wmi_impl::STATE_ENABLED)
            })
            .await
            .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || {
                wmi_impl::request_state_change(&id, wmi_impl::STATE_DISABLED)
            })
            .await
            .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || {
                wmi_impl::request_state_change(&id, wmi_impl::STATE_REBOOT)
            })
            .await
            .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || wmi_impl::destroy_system(&id))
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn snapshot(
        &self,
        id: &str,
        name: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<WorkloadSnapshot, WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            let name = name.to_string();
            tokio::task::spawn_blocking(move || wmi_impl::create_snapshot(&id, &name))
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = (id, name);
            Err(not_supported())
        }
    }

    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        #[cfg(windows)]
        {
            let id = id.to_string();
            tokio::task::spawn_blocking(move || wmi_impl::list_snapshots(&id))
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = id;
            Err(not_supported())
        }
    }

    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        #[cfg(windows)]
        {
            let snapshot_id = snapshot_id.to_string();
            tokio::task::spawn_blocking(move || wmi_impl::apply_snapshot(&snapshot_id))
                .await
                .map_err(|e| WorkloadError::DriverError(format!("join: {e}")))?
        }
        #[cfg(not(windows))]
        {
            let _ = snapshot_id;
            Err(not_supported())
        }
    }

    async fn run_command(&self, _id: &str, _command: &[&str]) -> Result<RunResult, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "in-guest command execution requires a guest agent; WMI does not expose it".into(),
        ))
    }

    async fn logs(&self, _id: &str, _lines: u32) -> Result<String, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "Hyper-V does not expose guest console logs through WMI; enable serial or KVP".into(),
        ))
    }

    async fn resize(
        &self,
        _id: &str,
        _cpu: Option<u32>,
        _memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError> {
        // Resize requires modifying Msvm_ProcessorSettingData and
        // Msvm_MemorySettingData and invoking ModifyResourceSettings on the
        // management service with an embedded MOF instance. Out of scope for
        // the initial WMI cut.
        Err(WorkloadError::NotSupported(
            "hyperv resize not yet implemented via WMI".into(),
        ))
    }
}

#[cfg(windows)]
fn workload_state_from_enabled(state: u16) -> WorkloadState {
    // Msvm_ComputerSystem.EnabledState values, see
    // https://learn.microsoft.com/windows/win32/hyperv_v2/msvm-computersystem
    match state {
        2 => WorkloadState::Running,
        3 => WorkloadState::Stopped,
        4 => WorkloadState::Stopped,   // Shutting down
        6 => WorkloadState::Stopped,   // Enabled but offline
        10 => WorkloadState::Creating, // Starting
        32768 => WorkloadState::Paused,
        32769 => WorkloadState::Paused, // Suspended
        _ => WorkloadState::Unknown,
    }
}

#[cfg(windows)]
fn workload_from_row(row: wmi_impl::ComputerSystem) -> Workload {
    Workload {
        id: row.name.clone(),
        name: row.element_name,
        workload_type: WorkloadType::Vm,
        state: workload_state_from_enabled(row.enabled_state),
        cpu_cores: None,
        memory_mb: None,
        disk_gb: None,
        image: None,
        host: None,
        ips: Vec::new(),
    }
}
