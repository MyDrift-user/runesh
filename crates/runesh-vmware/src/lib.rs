#![deny(unsafe_code)]
//! VMware vCenter/ESXi workload driver via REST API.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use runesh_workload::{
    RunResult, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot, WorkloadState,
    WorkloadType,
};

pub struct VmwareDriver {
    client: Client,
    base_url: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct VcVm {
    vm: String,
    name: String,
    power_state: String,
    #[serde(default)]
    cpu_count: Option<u32>,
    #[serde(default)]
    memory_size_MiB: Option<u64>,
}

impl VmwareDriver {
    pub async fn connect(
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, WorkloadError> {
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| WorkloadError::DriverError(format!("http: {e}")))?;
        let base_url = format!("https://{host}");

        let resp = client
            .post(format!("{base_url}/api/session"))
            .basic_auth(username, Some(password))
            .send()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("auth: {e}")))?;

        if !resp.status().is_success() {
            return Err(WorkloadError::DriverError("authentication failed".into()));
        }

        let session_id = resp
            .text()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("session: {e}")))?
            .trim_matches('"')
            .to_string();

        Ok(Self {
            client,
            base_url,
            session_id,
        })
    }

    fn map_power(state: &str) -> WorkloadState {
        match state {
            "POWERED_ON" => WorkloadState::Running,
            "POWERED_OFF" => WorkloadState::Stopped,
            "SUSPENDED" => WorkloadState::Paused,
            _ => WorkloadState::Unknown,
        }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("vmware-api-session-id", &self.session_id)
    }
}

#[async_trait]
impl WorkloadDriver for VmwareDriver {
    fn driver_name(&self) -> &str {
        "vmware"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        let vms: Vec<VcVm> = self
            .auth(self.client.get(format!("{}/api/vcenter/vm", self.base_url)))
            .send()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("list: {e}")))?
            .json()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("parse: {e}")))?;
        Ok(vms
            .into_iter()
            .map(|vm| Workload {
                id: vm.vm.clone(),
                name: vm.name,
                workload_type: WorkloadType::Vm,
                state: Self::map_power(&vm.power_state),
                cpu_cores: vm.cpu_count,
                memory_mb: vm.memory_size_MiB,
                disk_gb: None,
                image: None,
                host: None,
                ips: vec![],
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        let vm: VcVm = self
            .auth(
                self.client
                    .get(format!("{}/api/vcenter/vm/{id}", self.base_url)),
            )
            .send()
            .await
            .map_err(|e| WorkloadError::NotFound(format!("{id}: {e}")))?
            .json()
            .await
            .map_err(|e| WorkloadError::NotFound(format!("parse: {e}")))?;
        Ok(Workload {
            id: vm.vm,
            name: vm.name,
            workload_type: WorkloadType::Vm,
            state: Self::map_power(&vm.power_state),
            cpu_cores: vm.cpu_count,
            memory_mb: vm.memory_size_MiB,
            disk_gb: None,
            image: None,
            host: None,
            ips: vec![],
        })
    }

    async fn create(&self, _: &serde_json::Value) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VM creation requires complex config".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        self.auth(
            self.client
                .post(format!("{}/api/vcenter/vm/{id}/power/start", self.base_url)),
        )
        .send()
        .await
        .map_err(|e| WorkloadError::OperationFailed(format!("start: {e}")))?;
        Ok(())
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        self.auth(
            self.client
                .post(format!("{}/api/vcenter/vm/{id}/power/stop", self.base_url)),
        )
        .send()
        .await
        .map_err(|e| WorkloadError::OperationFailed(format!("stop: {e}")))?;
        Ok(())
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        self.auth(
            self.client
                .post(format!("{}/api/vcenter/vm/{id}/power/reset", self.base_url)),
        )
        .send()
        .await
        .map_err(|e| WorkloadError::OperationFailed(format!("reset: {e}")))?;
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        self.auth(
            self.client
                .delete(format!("{}/api/vcenter/vm/{id}", self.base_url)),
        )
        .send()
        .await
        .map_err(|e| WorkloadError::OperationFailed(format!("delete: {e}")))?;
        Ok(())
    }

    async fn snapshot(&self, _: &str, _: &str) -> Result<WorkloadSnapshot, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use vSphere API for snapshots".into(),
        ))
    }
    async fn list_snapshots(&self, _: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        Ok(vec![])
    }
    async fn restore_snapshot(&self, _: &str) -> Result<(), WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use vSphere API for snapshots".into(),
        ))
    }
    async fn run_command(&self, _: &str, _: &[&str]) -> Result<RunResult, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use VMware Tools for guest commands".into(),
        ))
    }
    async fn logs(&self, _: &str, _: u32) -> Result<String, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VMs don't have stdout logs".into(),
        ))
    }
    async fn resize(&self, _: &str, _: Option<u32>, _: Option<u64>) -> Result<(), WorkloadError> {
        Err(WorkloadError::NotSupported(
            "resize requires VM to be powered off".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_mapping() {
        assert_eq!(
            VmwareDriver::map_power("POWERED_ON"),
            WorkloadState::Running
        );
        assert_eq!(
            VmwareDriver::map_power("POWERED_OFF"),
            WorkloadState::Stopped
        );
        assert_eq!(VmwareDriver::map_power("SUSPENDED"), WorkloadState::Paused);
    }
}
