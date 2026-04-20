#![deny(unsafe_code)]
//! Uniform workload driver trait for VMs, containers, and Kubernetes.
//!
//! This crate defines the trait. Actual driver implementations live in
//! separate crates: runesh-docker, runesh-k8s, runesh-hyperv,
//! runesh-proxmox, runesh-vmware.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A workload (VM, container, or pod).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub id: String,
    pub name: String,
    pub workload_type: WorkloadType,
    pub state: WorkloadState,
    #[serde(default)]
    pub cpu_cores: Option<u32>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub disk_gb: Option<u64>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub ips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadType {
    Vm,
    Container,
    Pod,
    LxcContainer,
    Jail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadState {
    Running,
    Stopped,
    Paused,
    Creating,
    Migrating,
    Error,
    Unknown,
}

/// A snapshot of a workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadSnapshot {
    pub id: String,
    pub workload_id: String,
    pub name: String,
    pub created_at: String,
    pub size_bytes: Option<u64>,
}

/// Command execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Uniform driver trait for workload management.
#[async_trait]
pub trait WorkloadDriver: Send + Sync {
    fn driver_name(&self) -> &str;

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError>;
    async fn get(&self, id: &str) -> Result<Workload, WorkloadError>;
    async fn create(&self, spec: &serde_json::Value) -> Result<Workload, WorkloadError>;
    async fn start(&self, id: &str) -> Result<(), WorkloadError>;
    async fn stop(&self, id: &str) -> Result<(), WorkloadError>;
    async fn restart(&self, id: &str) -> Result<(), WorkloadError>;
    async fn destroy(&self, id: &str) -> Result<(), WorkloadError>;

    async fn snapshot(&self, id: &str, name: &str) -> Result<WorkloadSnapshot, WorkloadError>;
    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError>;
    async fn restore_snapshot(&self, snapshot_id: &str) -> Result<(), WorkloadError>;

    async fn run_command(&self, id: &str, command: &[&str]) -> Result<RunResult, WorkloadError>;
    async fn logs(&self, id: &str, lines: u32) -> Result<String, WorkloadError>;

    async fn resize(
        &self,
        id: &str,
        cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError>;
}

#[derive(Debug, thiserror::Error)]
pub enum WorkloadError {
    #[error("workload not found: {0}")]
    NotFound(String),
    #[error("operation failed: {0}")]
    OperationFailed(String),
    #[error("driver error: {0}")]
    DriverError(String),
    #[error("not supported: {0}")]
    NotSupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_serialization() {
        let w = Workload {
            id: "vm-1".into(),
            name: "web-server".into(),
            workload_type: WorkloadType::Vm,
            state: WorkloadState::Running,
            cpu_cores: Some(4),
            memory_mb: Some(8192),
            disk_gb: Some(100),
            image: Some("ubuntu-22.04".into()),
            host: Some("hv-01".into()),
            ips: vec!["10.0.0.5".into()],
        };
        let json = serde_json::to_string(&w).unwrap();
        let parsed: Workload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "web-server");
        assert_eq!(parsed.state, WorkloadState::Running);
    }

    #[test]
    fn all_workload_types() {
        for wt in [
            WorkloadType::Vm,
            WorkloadType::Container,
            WorkloadType::Pod,
            WorkloadType::LxcContainer,
            WorkloadType::Jail,
        ] {
            let json = serde_json::to_string(&wt).unwrap();
            let parsed: WorkloadType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, wt);
        }
    }

    #[test]
    fn all_states() {
        for s in [
            WorkloadState::Running,
            WorkloadState::Stopped,
            WorkloadState::Paused,
            WorkloadState::Creating,
            WorkloadState::Migrating,
            WorkloadState::Error,
            WorkloadState::Unknown,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let parsed: WorkloadState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn snapshot_serialization() {
        let snap = WorkloadSnapshot {
            id: "s1".into(),
            workload_id: "vm-1".into(),
            name: "before-upgrade".into(),
            created_at: "2026-04-20T00:00:00Z".into(),
            size_bytes: Some(1024 * 1024 * 1024),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: WorkloadSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "before-upgrade");
    }
}
