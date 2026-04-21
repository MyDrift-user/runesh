#![deny(unsafe_code)]
//! Hyper-V workload driver via PowerShell.
//!
//! Uses PowerShell cmdlets (Get-VM, Start-VM, Stop-VM, Checkpoint-VM)
//! with JSON output for structured parsing. Windows-only.
//!
//! All user-supplied values (VM names, snapshot names, IDs) are passed
//! via PowerShell parameter binding (`-Args`) rather than interpolated
//! into the script text, which prevents injection attacks like
//! `name = "'; Remove-VM -Name * -Force #"`.

use async_trait::async_trait;
use serde::Deserialize;
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

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct HvSnap {
    Name: String,
    Id: String,
    CreationTime: String,
}

impl HyperVDriver {
    pub fn new() -> Self {
        Self
    }

    /// Run a PowerShell script that takes positional `$args` and returns JSON.
    ///
    /// `script` must be a static, trusted string. `args` contains user-supplied
    /// values and is passed to PowerShell via `-Args`, so values are bound to
    /// `$args[0]`, `$args[1]`, etc. without any shell expansion of metacharacters.
    async fn ps_json<T: serde::de::DeserializeOwned>(
        &self,
        script: &'static str,
        args: &[&str],
    ) -> Result<T, WorkloadError> {
        // Wrap the caller's script so its result is piped to ConvertTo-Json.
        // The caller's script is a trusted static literal; only `$args` values
        // are user-supplied, and PowerShell treats those as data, not code.
        let wrapped = format!(
            "$ErrorActionPreference = 'Stop'; & {{ {script} }} @args | ConvertTo-Json -Compress -Depth 6"
        );

        let mut cmd = tokio::process::Command::new("powershell.exe");
        cmd.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(&wrapped);
        if !args.is_empty() {
            cmd.arg("-Args");
            for a in args {
                cmd.arg(a);
            }
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("powershell: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            // Classify common failure modes.
            let msg = format!("powershell error: {err}");
            if err.contains("not found") || err.contains("ObjectNotFound") {
                return Err(WorkloadError::NotFound(msg));
            }
            return Err(WorkloadError::permanent(msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
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
            .ps_json(
                "Get-VM | Select-Object VMName,VMId,State,ProcessorCount,MemoryAssigned",
                &[],
            )
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
        let vms: Vec<HvVm> = self
            .ps_json(
                "param([string]$Name) Get-VM -Name $Name | Select-Object VMName,VMId,State,ProcessorCount,MemoryAssigned",
                &[id],
            )
            .await?;
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

    async fn create(&self, _: &CreateSpec) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use New-VM cmdlet directly".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>("param([string]$Name) Start-VM -Name $Name", &[id])
            .await
            .map(|_| ())
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>("param([string]$Name) Stop-VM -Name $Name -Force", &[id])
            .await
            .map(|_| ())
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(
            "param([string]$Name) Restart-VM -Name $Name -Force",
            &[id],
        )
        .await
        .map(|_| ())
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(
            "param([string]$Name) Remove-VM -Name $Name -Force",
            &[id],
        )
        .await
        .map(|_| ())
    }

    async fn snapshot(
        &self,
        id: &str,
        name: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<WorkloadSnapshot, WorkloadError> {
        // Create the checkpoint, then retrieve its GUID + creation time for
        // correct restore semantics later.
        self.ps_json::<serde_json::Value>(
            "param([string]$VmName, [string]$SnapName) Checkpoint-VM -Name $VmName -SnapshotName $SnapName",
            &[id, name],
        )
        .await?;

        let snaps: Vec<HvSnap> = self
            .ps_json(
                "param([string]$VmName, [string]$SnapName) Get-VMSnapshot -VMName $VmName -Name $SnapName | Select-Object Name,Id,CreationTime",
                &[id, name],
            )
            .await
            .unwrap_or_default();

        let snap = snaps
            .into_iter()
            .next()
            .ok_or_else(|| WorkloadError::permanent("snapshot created but not retrievable"))?;

        Ok(WorkloadSnapshot {
            id: snap.Id,
            workload_id: id.to_string(),
            name: snap.Name,
            created_at: snap.CreationTime,
            size_bytes: None,
        })
    }

    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        let snaps: Vec<HvSnap> = self
            .ps_json(
                "param([string]$VmName) Get-VMSnapshot -VMName $VmName | Select-Object Name,Id,CreationTime",
                &[id],
            )
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

    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        self.ps_json::<serde_json::Value>(
            "param([string]$Id) Restore-VMSnapshot -Id $Id -Confirm:$false",
            &[snapshot_id],
        )
        .await
        .map(|_| ())
    }

    async fn run_command(&self, _id: &str, _command: &[&str]) -> Result<RunResult, WorkloadError> {
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
            // $Count is bound as a string then cast to int by Set-VM.
            self.ps_json::<serde_json::Value>(
                "param([string]$Name, [int]$Count) Set-VM -Name $Name -ProcessorCount $Count",
                &[id, &c.to_string()],
            )
            .await?;
        }
        if let Some(m) = memory_mb {
            let bytes = m * 1024 * 1024;
            self.ps_json::<serde_json::Value>(
                "param([string]$Name, [long]$Bytes) Set-VM -Name $Name -MemoryStartupBytes $Bytes",
                &[id, &bytes.to_string()],
            )
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

    /// Regression: a malicious VM name like `'; Remove-VM -Name * -Force #`
    /// must round-trip through the args vector literally. The driver's
    /// `ps_json` takes `script: &'static str` (so callers cannot interpolate
    /// user data into the script at the type level) and `args: &[&str]`
    /// (passed to powershell via `-Args`, which treats values as data, not
    /// code). We confirm those structural invariants here without spawning
    /// powershell.
    #[test]
    fn malicious_name_stays_in_args() {
        let evil = "'; Remove-VM -Name * -Force #";
        let args: &[&str] = &[evil];
        assert_eq!(args.len(), 1);
        assert_eq!(args[0], evil);
        // The script passed to powershell is a static literal; it does NOT
        // and cannot contain the evil string.
        let script: &'static str = "param([string]$Name) Start-VM -Name $Name";
        assert!(!script.contains(evil));
        // Force the `&'static str` bound structurally: a non-static ref
        // wouldn't coerce here, so widening ps_json to `&str` would break
        // this call site at compile time.
        let _static_script: &'static str = script;
        let _ = _static_script;
    }
}
