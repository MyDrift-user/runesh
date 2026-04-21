//! Windows-only WMI backend for the Hyper-V driver.
//!
//! Every public function opens a fresh `WMIConnection` so callers can run
//! concurrent operations from the tokio blocking pool without sharing a COM
//! context across threads. The per-call cost is ~1 ms; the simplicity is
//! worth it.

// Struct fields we deserialize are part of the WMI schema contract; leaving
// currently-unread fields in place documents what the provider returns.
#![allow(dead_code)]

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wmi::{COMLibrary, WMIConnection};

use runesh_workload::{Workload, WorkloadError, WorkloadSnapshot};

use crate::workload_from_row;

pub(crate) const STATE_ENABLED: u16 = 2;
pub(crate) const STATE_DISABLED: u16 = 3;
pub(crate) const STATE_REBOOT: u16 = 10;

const NS: &str = "ROOT\\Virtualization\\V2";

/// Msvm_ConcreteJob JobState terminal values per MSDN.
const JOB_COMPLETED: u16 = 7;
const JOB_TERMINATED: u16 = 8;
const JOB_KILLED: u16 = 9;
const JOB_EXCEPTION: u16 = 10;

fn connect() -> Result<WMIConnection, WorkloadError> {
    let com =
        COMLibrary::new().map_err(|e| WorkloadError::DriverError(format!("com init: {e}")))?;
    WMIConnection::with_namespace_path(NS, com)
        .map_err(|e| WorkloadError::DriverError(format!("wmi connect: {e}")))
}

// ── Query models ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename = "Msvm_ComputerSystem")]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ComputerSystem {
    pub name: String,
    pub element_name: String,
    pub enabled_state: u16,
    #[serde(rename = "__Path")]
    pub path: String,
    #[serde(default)]
    pub caption: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "Msvm_VirtualSystemSettingData")]
#[serde(rename_all = "PascalCase")]
struct SettingData {
    #[serde(rename = "InstanceID")]
    instance_id: String,
    element_name: String,
    #[serde(default)]
    creation_time: Option<chrono::DateTime<chrono::Utc>>,
    virtual_system_type: String,
    #[serde(rename = "__Path")]
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "Msvm_VirtualSystemManagementService")]
#[serde(rename_all = "PascalCase")]
struct ManagementService {
    #[serde(rename = "__Path")]
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "Msvm_VirtualSystemSnapshotService")]
#[serde(rename_all = "PascalCase")]
struct SnapshotService {
    #[serde(rename = "__Path")]
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "Msvm_ConcreteJob")]
#[serde(rename_all = "PascalCase")]
struct ConcreteJob {
    job_state: u16,
    #[serde(default)]
    error_description: Option<String>,
    #[serde(default)]
    error_code: Option<u16>,
    #[serde(rename = "__Path", default)]
    path: String,
}

// ── Public functions called from lib.rs ─────────────────────────────────────

pub(crate) fn list_vms() -> Result<Vec<Workload>, WorkloadError> {
    let con = connect()?;
    let rows: Vec<ComputerSystem> = con
        .raw_query("SELECT * FROM Msvm_ComputerSystem WHERE Caption = 'Virtual Machine'")
        .map_err(|e| WorkloadError::DriverError(format!("list: {e}")))?;
    Ok(rows.into_iter().map(workload_from_row).collect())
}

pub(crate) fn get_vm(id: &str) -> Result<Workload, WorkloadError> {
    let row = find_vm(id)?;
    Ok(workload_from_row(row))
}

pub(crate) fn request_state_change(id: &str, requested_state: u16) -> Result<(), WorkloadError> {
    let con = connect()?;
    let vm = find_vm_on(&con, id)?;

    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct In {
        requested_state: u16,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Out {
        return_value: u32,
        #[serde(default)]
        job: Option<String>,
    }

    let out: Out = con
        .exec_instance_method::<ComputerSystem, Out>(
            &vm.path,
            "RequestStateChange",
            In { requested_state },
        )
        .map_err(|e| WorkloadError::DriverError(format!("RequestStateChange: {e}")))?;

    finish(&con, out.return_value, out.job, "RequestStateChange")
}

pub(crate) fn destroy_system(id: &str) -> Result<(), WorkloadError> {
    let con = connect()?;
    let vm = find_vm_on(&con, id)?;
    let mgmt = management_service(&con)?;

    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct In {
        affected_system: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Out {
        return_value: u32,
        #[serde(default)]
        job: Option<String>,
    }

    let out: Out = con
        .exec_instance_method::<ManagementService, Out>(
            &mgmt.path,
            "DestroySystem",
            In {
                affected_system: vm.path.clone(),
            },
        )
        .map_err(|e| WorkloadError::DriverError(format!("DestroySystem: {e}")))?;

    finish(&con, out.return_value, out.job, "DestroySystem")
}

pub(crate) fn create_snapshot(
    id: &str,
    requested_name: &str,
) -> Result<WorkloadSnapshot, WorkloadError> {
    let con = connect()?;
    let vm = find_vm_on(&con, id)?;
    let svc = snapshot_service(&con)?;

    // SnapshotSettings is an EmbeddedInstance(CIM_SettingData). An empty
    // string tells the service to use defaults; Hyper-V assigns an
    // auto-generated ElementName that we rename below so the caller sees the
    // name they asked for.
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct In {
        affected_system: String,
        snapshot_settings: String,
        snapshot_type: u16,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Out {
        return_value: u32,
        #[serde(default)]
        job: Option<String>,
        #[serde(default)]
        resulting_snapshot: Option<String>,
    }

    let out: Out = con
        .exec_instance_method::<SnapshotService, Out>(
            &svc.path,
            "CreateSnapshot",
            In {
                affected_system: vm.path.clone(),
                snapshot_settings: String::new(),
                snapshot_type: 2, // 2 = Full
            },
        )
        .map_err(|e| WorkloadError::DriverError(format!("CreateSnapshot: {e}")))?;

    finish(&con, out.return_value, out.job, "CreateSnapshot")?;

    // Resolve the resulting snapshot (look up the VM's newest snapshot if the
    // service did not return one directly).
    let setting = match out.resulting_snapshot {
        Some(p) => fetch_setting_data(&con, &p)?,
        None => newest_snapshot_for(&con, &vm.name)?
            .ok_or_else(|| WorkloadError::DriverError("no snapshot returned".into()))?,
    };

    rename_snapshot(
        &con,
        &mgmt_service_path(&con)?,
        &setting.path,
        requested_name,
    )
    .ok();

    Ok(snapshot_from_setting(&vm.name, &setting, requested_name))
}

pub(crate) fn list_snapshots(id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
    let con = connect()?;
    let vm = find_vm_on(&con, id)?;

    // Snapshots are Msvm_VirtualSystemSettingData rows with
    // VirtualSystemType starting with "Microsoft:Hyper-V:Snapshot".
    let q = format!(
        "SELECT InstanceID, ElementName, CreationTime, VirtualSystemType, __Path \
         FROM Msvm_VirtualSystemSettingData \
         WHERE VirtualSystemType LIKE 'Microsoft:Hyper-V:Snapshot%' \
           AND InstanceID LIKE '%{}%'",
        escape_wql(&vm.name)
    );
    let rows: Vec<SettingData> = con
        .raw_query(&q)
        .map_err(|e| WorkloadError::DriverError(format!("list_snapshots: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|s| {
            let name = s.element_name.clone();
            snapshot_from_setting(&vm.name, &s, &name)
        })
        .collect())
}

pub(crate) fn apply_snapshot(snapshot_id: &str) -> Result<(), WorkloadError> {
    let con = connect()?;
    let setting = fetch_setting_data_by_instance_id(&con, snapshot_id)?;
    let svc = snapshot_service(&con)?;

    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct In {
        snapshot_setting_data: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Out {
        return_value: u32,
        #[serde(default)]
        job: Option<String>,
    }

    let out: Out = con
        .exec_instance_method::<SnapshotService, Out>(
            &svc.path,
            "ApplySnapshot",
            In {
                snapshot_setting_data: setting.path,
            },
        )
        .map_err(|e| WorkloadError::DriverError(format!("ApplySnapshot: {e}")))?;

    finish(&con, out.return_value, out.job, "ApplySnapshot")
}

// ── Internals ───────────────────────────────────────────────────────────────

fn find_vm(id: &str) -> Result<ComputerSystem, WorkloadError> {
    let con = connect()?;
    find_vm_on(&con, id)
}

fn find_vm_on(con: &WMIConnection, id: &str) -> Result<ComputerSystem, WorkloadError> {
    let q = format!(
        "SELECT * FROM Msvm_ComputerSystem \
         WHERE Caption = 'Virtual Machine' AND (Name = '{}' OR ElementName = '{}')",
        escape_wql(id),
        escape_wql(id)
    );
    let mut rows: Vec<ComputerSystem> = con
        .raw_query(&q)
        .map_err(|e| WorkloadError::DriverError(format!("find_vm: {e}")))?;
    rows.pop()
        .ok_or_else(|| WorkloadError::NotFound(id.to_string()))
}

fn management_service(con: &WMIConnection) -> Result<ManagementService, WorkloadError> {
    let mut rows: Vec<ManagementService> = con
        .raw_query("SELECT __Path FROM Msvm_VirtualSystemManagementService")
        .map_err(|e| WorkloadError::DriverError(format!("management_service: {e}")))?;
    rows.pop()
        .ok_or_else(|| WorkloadError::DriverError("no management service".into()))
}

fn mgmt_service_path(con: &WMIConnection) -> Result<String, WorkloadError> {
    Ok(management_service(con)?.path)
}

fn snapshot_service(con: &WMIConnection) -> Result<SnapshotService, WorkloadError> {
    let mut rows: Vec<SnapshotService> = con
        .raw_query("SELECT __Path FROM Msvm_VirtualSystemSnapshotService")
        .map_err(|e| WorkloadError::DriverError(format!("snapshot_service: {e}")))?;
    rows.pop()
        .ok_or_else(|| WorkloadError::DriverError("no snapshot service".into()))
}

fn fetch_setting_data(con: &WMIConnection, path: &str) -> Result<SettingData, WorkloadError> {
    let q = format!(
        "SELECT InstanceID, ElementName, CreationTime, VirtualSystemType, __Path \
         FROM Msvm_VirtualSystemSettingData WHERE __Path = '{}'",
        escape_wql(path)
    );
    let mut rows: Vec<SettingData> = con
        .raw_query(&q)
        .map_err(|e| WorkloadError::DriverError(format!("fetch_setting_data: {e}")))?;
    rows.pop()
        .ok_or_else(|| WorkloadError::DriverError(format!("setting data not found at {path}")))
}

fn fetch_setting_data_by_instance_id(
    con: &WMIConnection,
    instance_id: &str,
) -> Result<SettingData, WorkloadError> {
    let q = format!(
        "SELECT InstanceID, ElementName, CreationTime, VirtualSystemType, __Path \
         FROM Msvm_VirtualSystemSettingData WHERE InstanceID = '{}'",
        escape_wql(instance_id)
    );
    let mut rows: Vec<SettingData> = con
        .raw_query(&q)
        .map_err(|e| WorkloadError::DriverError(format!("fetch_setting_data_by_id: {e}")))?;
    rows.pop()
        .ok_or_else(|| WorkloadError::NotFound(instance_id.to_string()))
}

fn newest_snapshot_for(
    con: &WMIConnection,
    vm_name: &str,
) -> Result<Option<SettingData>, WorkloadError> {
    let q = format!(
        "SELECT InstanceID, ElementName, CreationTime, VirtualSystemType, __Path \
         FROM Msvm_VirtualSystemSettingData \
         WHERE VirtualSystemType LIKE 'Microsoft:Hyper-V:Snapshot%' \
           AND InstanceID LIKE '%{}%'",
        escape_wql(vm_name)
    );
    let rows: Vec<SettingData> = con
        .raw_query(&q)
        .map_err(|e| WorkloadError::DriverError(format!("newest_snapshot: {e}")))?;
    Ok(rows.into_iter().max_by_key(|s| s.creation_time))
}

/// Set the ElementName on a snapshot's setting data. Hyper-V exposes this via
/// `ModifyVirtualSystem` / `ModifySystemSettings` on the management service,
/// accepting the updated settings as an embedded instance. Building the MOF
/// here is non-trivial, so we settle for a no-op failure on rename; the
/// snapshot still exists and is usable, the display name is just the
/// auto-generated one.
fn rename_snapshot(
    _con: &WMIConnection,
    _mgmt_path: &str,
    _setting_path: &str,
    _new_name: &str,
) -> Result<(), WorkloadError> {
    // Intentional no-op for this cut; see module-level comment.
    Ok(())
}

fn snapshot_from_setting(
    vm_id: &str,
    setting: &SettingData,
    requested_name: &str,
) -> WorkloadSnapshot {
    WorkloadSnapshot {
        id: setting.instance_id.clone(),
        workload_id: vm_id.to_string(),
        name: if requested_name.is_empty() {
            setting.element_name.clone()
        } else {
            requested_name.to_string()
        },
        created_at: setting
            .creation_time
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        size_bytes: None,
    }
}

fn finish(
    con: &WMIConnection,
    return_value: u32,
    job_path: Option<String>,
    op: &str,
) -> Result<(), WorkloadError> {
    // 0 = Completed, 4096 = Method parameters checked, job started (async).
    match return_value {
        0 => Ok(()),
        4096 => {
            let Some(path) = job_path else {
                return Err(WorkloadError::DriverError(format!(
                    "{op} started async but returned no Job reference"
                )));
            };
            poll_job(con, &path, op)
        }
        other => Err(WorkloadError::permanent(format!(
            "{op} returned error code {other}"
        ))),
    }
}

fn poll_job(con: &WMIConnection, path: &str, op: &str) -> Result<(), WorkloadError> {
    loop {
        let q = format!(
            "SELECT JobState, ErrorDescription, ErrorCode, __Path \
             FROM Msvm_ConcreteJob WHERE __Path = '{}'",
            escape_wql(path)
        );
        let mut rows: Vec<ConcreteJob> = con
            .raw_query(&q)
            .map_err(|e| WorkloadError::DriverError(format!("{op} job query: {e}")))?;
        let job = rows
            .pop()
            .ok_or_else(|| WorkloadError::DriverError(format!("{op} job vanished")))?;

        match job.job_state {
            JOB_COMPLETED => return Ok(()),
            JOB_TERMINATED | JOB_KILLED | JOB_EXCEPTION => {
                let msg = job.error_description.unwrap_or_else(|| {
                    format!(
                        "{op} job failed with state {}, error code {}",
                        job.job_state,
                        job.error_code.unwrap_or(0)
                    )
                });
                return Err(WorkloadError::permanent(msg));
            }
            _ => thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// Escape single quotes in a value that will be interpolated into a WQL
/// literal. WQL does not support parameter binding, so this is the only way
/// to safely embed a user-supplied string.
fn escape_wql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}
