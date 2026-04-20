//! Hyper-V workload driver via PowerShell.
//!
//! Uses PowerShell cmdlets (Get-VM, Start-VM, Stop-VM, Checkpoint-VM)
//! with JSON output for structured parsing. Windows-only.

use async_trait::async_trait;
use serde::Deserialize;

use runesh_workload::{
    RunResult, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot, WorkloadState,
    WorkloadType,
};

pub struct HyperVDriver;

#[derive(Debug, Deserialize)]
struct HvVm {
    #[serde(alias = "VMName", alias = "Name")]
    name: String,
    #[serde(alias = "VMId", alias = "Id")]
    id: String,
    #[serde(alias = "State")]
    state: u32,
    #[serde(alias = "ProcessorCount", default)]
    processor_count: Option<u32>,
    #[serde(alias = "MemoryAssigned", default)]
    memory_assigned: Option<u64>,
}

impl HyperVDriver {
    pub fn new() -> Self {
        Self
    }

    async fn ps_json<T: serde::de::DeserializeOwned>(
        &self,
        script: &str,
    ) -> Result<T, WorkloadError> {
        let output = tokio::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("{script} | ConvertTo-Json -Compress"),
            ])
            .output()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("powershell: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(WorkloadError::DriverError(format!(
                "powershell error: {err}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // PowerShell may return a single object (not array) when there's one result
        // Wrap in array if needed
        let json = stdout.trim();
        if json.is_empty() || json == "null" {
            return serde_json::from_str("[]")
                .map_err(|e| WorkloadError::DriverError(format!("parse: {e}")));
        }

        serde_json::from_str(json)
            .or_else(|_| serde_json::from_str(&format!("[{json}]")))
            .map_err(|e| WorkloadError::DriverError(format!("json parse: {e}")))
    }

    fn map_state(state: u32) -> WorkloadState {
        // Hyper-V VM states: 2=Running, 3=Off, 6=Saved, 9=Paused, 32768=Starting
        match state {
            2 => WorkloadState::Running,
            3 | 6 => WorkloadState::Stopped,
            9 => WorkloadState::Paused,
            32768 | 32770 => WorkloadState::Creating,
            _ => WorkloadState::Unknown,
        }
    }
}

#[async_trait]
impl WorkloadDriver for HyperVDriver {
    fn driver_name(&self) -> &str {
        "hyperv"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        let vms: Vec<HvVm> = self
            .ps_json("Get-VM | Select-Object VMName,VMId,State,ProcessorCount,MemoryAssigned")
            .await?;
        Ok(vms
            .into_iter()
            .map(|vm| Workload {
                id: vm.id,
                name: vm.name,
                workload_type: WorkloadType::Vm,
                state: Self::map_state(vm.state),
                cpu_cores: vm.processor_count,
                memory_mb: vm.memory_assigned.map(|m| m / 1024 / 1024),
                disk_gb: None,
                image: None,
                host: None,
                ips: vec![],
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        let vms: Vec<HvVm> = self.ps_json(&format!(
            "Get-VM -Name '{id}' | Select-Object VMName,VMId,State,ProcessorCount,MemoryAssigned"
        )).await?;
        vms.into_iter()
            .next()
            .map(|vm| Workload {
                id: vm.id,
                name: vm.name,
                workload_type: WorkloadType::Vm,
                state: Self::map_state(vm.state),
                cpu_cores: vm.processor_count,
                memory_mb: vm.memory_assigned.map(|m| m / 1024 / 1024),
                disk_gb: None,
                image: None,
                host: None,
                ips: vec![],
            })
            .ok_or_else(|| WorkloadError::NotFound(id.into()))
    }

    async fn create(&self, _: &serde_json::Value) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use New-VM cmdlet directly".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!("Start-VM -Name '{id}'"))
            .await
            .map(|_| ())
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!("Stop-VM -Name '{id}' -Force"))
            .await
            .map(|_| ())
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!("Restart-VM -Name '{id}' -Force"))
            .await
            .map(|_| ())
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!("Remove-VM -Name '{id}' -Force"))
            .await
            .map(|_| ())
    }

    async fn snapshot(&self, id: &str, name: &str) -> Result<WorkloadSnapshot, WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!(
            "Checkpoint-VM -Name '{id}' -SnapshotName '{name}'"
        ))
        .await?;
        Ok(WorkloadSnapshot {
            id: name.to_string(),
            workload_id: id.to_string(),
            name: name.to_string(),
            created_at: String::new(),
            size_bytes: None,
        })
    }

    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        #[derive(Deserialize)]
        struct HvSnap {
            Name: String,
            Id: String,
            CreationTime: String,
        }
        let snaps: Vec<HvSnap> = self
            .ps_json(&format!(
                "Get-VMSnapshot -VMName '{id}' | Select-Object Name,Id,CreationTime"
            ))
            .await
            .unwrap_or_default();
        Ok(snaps
            .into_iter()
            .map(|s| WorkloadSnapshot {
                id: s.Id,
                workload_id: id.to_string(),
                name: s.Name,
                created_at: s.CreationTime,
                size_bytes: None,
            })
            .collect())
    }

    async fn restore_snapshot(&self, snapshot_id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(&format!(
            "Restore-VMSnapshot -Id '{snapshot_id}' -Confirm:$false"
        ))
        .await
        .map(|_| ())
    }

    async fn run_command(&self, id: &str, command: &[&str]) -> Result<RunResult, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use PowerShell Direct or VMConnect".into(),
        ))
    }

    async fn logs(&self, _: &str, _: u32) -> Result<String, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VMs don't have stdout logs".into(),
        ))
    }

    async fn resize(
        &self,
        id: &str,
        cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError> {
        if let Some(c) = cpu {
            self.ps_json::<serde_json::Value>(&format!("Set-VM -Name '{id}' -ProcessorCount {c}"))
                .await?;
        }
        if let Some(m) = memory_mb {
            let bytes = m * 1024 * 1024;
            self.ps_json::<serde_json::Value>(&format!(
                "Set-VM -Name '{id}' -MemoryStartupBytes {bytes}"
            ))
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_mapping() {
        assert_eq!(HyperVDriver::map_state(2), WorkloadState::Running);
        assert_eq!(HyperVDriver::map_state(3), WorkloadState::Stopped);
        assert_eq!(HyperVDriver::map_state(9), WorkloadState::Paused);
        assert_eq!(HyperVDriver::map_state(32768), WorkloadState::Creating);
    }
}
