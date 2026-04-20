//! Proxmox VE workload driver via REST API.
//!
//! Manages QEMU VMs and LXC containers on Proxmox VE clusters.
//! Authenticates via API token (recommended) or ticket-based auth.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use runesh_workload::{
    RunResult, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot, WorkloadState,
    WorkloadType,
};

/// Proxmox VE connection.
pub struct ProxmoxDriver {
    client: Client,
    base_url: String,
    node: String,
    auth: ProxmoxAuth,
}

enum ProxmoxAuth {
    Token { id: String, secret: String },
    Ticket { ticket: String, csrf: String },
}

/// Proxmox VM/CT status from API.
#[derive(Debug, Deserialize)]
struct PveVm {
    vmid: u64,
    name: Option<String>,
    status: String,
    #[serde(default)]
    cpus: Option<u32>,
    #[serde(default)]
    maxmem: Option<u64>,
    #[serde(default)]
    maxdisk: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PveResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct PveSnapshot {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    snaptime: Option<u64>,
}

impl ProxmoxDriver {
    /// Connect with API token (recommended).
    /// token_id format: "user@realm!tokenid"
    pub fn with_token(
        host: &str,
        node: &str,
        token_id: &str,
        token_secret: &str,
    ) -> Result<Self, WorkloadError> {
        let client = Client::builder()
            .danger_accept_invalid_certs(true) // self-signed certs common
            .build()
            .map_err(|e| WorkloadError::DriverError(format!("http client: {e}")))?;
        Ok(Self {
            client,
            base_url: format!("https://{host}:8006/api2/json"),
            node: node.to_string(),
            auth: ProxmoxAuth::Token {
                id: token_id.to_string(),
                secret: token_secret.to_string(),
            },
        })
    }

    /// Authenticate with username/password (returns ticket).
    pub async fn with_password(
        host: &str,
        node: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, WorkloadError> {
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| WorkloadError::DriverError(format!("http client: {e}")))?;
        let base_url = format!("https://{host}:8006/api2/json");

        let resp: PveResponse<serde_json::Value> = client
            .post(format!("{base_url}/access/ticket"))
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("auth: {e}")))?
            .json()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("auth parse: {e}")))?;

        let ticket = resp.data["ticket"].as_str().unwrap_or("").to_string();
        let csrf = resp.data["CSRFPreventionToken"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(Self {
            client,
            base_url,
            node: node.to_string(),
            auth: ProxmoxAuth::Ticket { ticket, csrf },
        })
    }

    fn auth_request(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            ProxmoxAuth::Token { id, secret } => {
                req.header("Authorization", format!("PVEAPIToken={id}={secret}"))
            }
            ProxmoxAuth::Ticket { ticket, csrf } => req
                .header("Cookie", format!("PVEAuthCookie={ticket}"))
                .header("CSRFPreventionToken", csrf),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, WorkloadError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp: PveResponse<T> = self
            .auth_request(self.client.get(&url))
            .send()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("GET {path}: {e}")))?
            .json()
            .await
            .map_err(|e| WorkloadError::DriverError(format!("parse {path}: {e}")))?;
        Ok(resp.data)
    }

    async fn post_action(&self, path: &str) -> Result<(), WorkloadError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .auth_request(self.client.post(&url))
            .send()
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("POST {path}: {e}")))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(WorkloadError::OperationFailed(format!("{path}: {body}")));
        }
        Ok(())
    }

    fn map_status(status: &str) -> WorkloadState {
        match status {
            "running" => WorkloadState::Running,
            "stopped" => WorkloadState::Stopped,
            "paused" => WorkloadState::Paused,
            _ => WorkloadState::Unknown,
        }
    }

    fn vm_path(&self, vmid: &str, suffix: &str) -> String {
        format!("nodes/{}/qemu/{}{}", self.node, vmid, suffix)
    }
}

#[async_trait]
impl WorkloadDriver for ProxmoxDriver {
    fn driver_name(&self) -> &str {
        "proxmox"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        let vms: Vec<PveVm> = self.get(&format!("nodes/{}/qemu", self.node)).await?;
        Ok(vms
            .into_iter()
            .map(|vm| Workload {
                id: vm.vmid.to_string(),
                name: vm.name.unwrap_or_else(|| format!("VM {}", vm.vmid)),
                workload_type: WorkloadType::Vm,
                state: Self::map_status(&vm.status),
                cpu_cores: vm.cpus,
                memory_mb: vm.maxmem.map(|m| m / 1024 / 1024),
                disk_gb: vm.maxdisk.map(|d| d / 1024 / 1024 / 1024),
                image: None,
                host: Some(self.node.clone()),
                ips: vec![],
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        let vm: PveVm = self.get(&self.vm_path(id, "/status/current")).await?;
        Ok(Workload {
            id: vm.vmid.to_string(),
            name: vm.name.unwrap_or_else(|| format!("VM {}", vm.vmid)),
            workload_type: WorkloadType::Vm,
            state: Self::map_status(&vm.status),
            cpu_cores: vm.cpus,
            memory_mb: vm.maxmem.map(|m| m / 1024 / 1024),
            disk_gb: vm.maxdisk.map(|d| d / 1024 / 1024 / 1024),
            image: None,
            host: Some(self.node.clone()),
            ips: vec![],
        })
    }

    async fn create(&self, _spec: &serde_json::Value) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VM creation requires complex config; use Proxmox UI or API directly".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        self.post_action(&self.vm_path(id, "/status/start")).await
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        self.post_action(&self.vm_path(id, "/status/stop")).await
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        self.post_action(&self.vm_path(id, "/status/reboot")).await
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        self.post_action(&format!("nodes/{}/qemu/{}", self.node, id))
            .await
    }

    async fn snapshot(&self, id: &str, name: &str) -> Result<WorkloadSnapshot, WorkloadError> {
        let url = format!("{}/{}", self.base_url, self.vm_path(id, "/snapshot"));
        let resp = self
            .auth_request(self.client.post(&url))
            .form(&[
                ("snapname", name),
                ("description", &format!("Created by runesh")),
            ])
            .send()
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("snapshot: {e}")))?;
        if !resp.status().is_success() {
            return Err(WorkloadError::OperationFailed("snapshot failed".into()));
        }
        Ok(WorkloadSnapshot {
            id: name.to_string(),
            workload_id: id.to_string(),
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            size_bytes: None,
        })
    }

    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        let snaps: Vec<PveSnapshot> = self.get(&self.vm_path(id, "/snapshot")).await?;
        Ok(snaps
            .into_iter()
            .filter(|s| s.name != "current")
            .map(|s| WorkloadSnapshot {
                id: s.name.clone(),
                workload_id: id.to_string(),
                name: s.name,
                created_at: s.snaptime.map(|t| t.to_string()).unwrap_or_default(),
                size_bytes: None,
            })
            .collect())
    }

    async fn restore_snapshot(&self, snapshot_id: &str) -> Result<(), WorkloadError> {
        // snapshot_id format: "vmid:snapname"
        let parts: Vec<&str> = snapshot_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(WorkloadError::OperationFailed(
                "snapshot_id format: vmid:snapname".into(),
            ));
        }
        self.post_action(&self.vm_path(parts[0], &format!("/snapshot/{}/rollback", parts[1])))
            .await
    }

    async fn run_command(&self, _id: &str, _command: &[&str]) -> Result<RunResult, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use QEMU guest agent for in-VM commands".into(),
        ))
    }

    async fn logs(&self, _id: &str, _lines: u32) -> Result<String, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VMs don't have stdout logs; use serial console".into(),
        ))
    }

    async fn resize(
        &self,
        id: &str,
        cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError> {
        let url = format!("{}/{}", self.base_url, self.vm_path(id, "/config"));
        let mut form = Vec::new();
        if let Some(c) = cpu {
            form.push(("cores", c.to_string()));
        }
        if let Some(m) = memory_mb {
            form.push(("memory", m.to_string()));
        }
        if form.is_empty() {
            return Ok(());
        }

        let resp = self
            .auth_request(self.client.put(&url))
            .form(&form)
            .send()
            .await
            .map_err(|e| WorkloadError::OperationFailed(format!("resize: {e}")))?;
        if !resp.status().is_success() {
            return Err(WorkloadError::OperationFailed("resize failed".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(ProxmoxDriver::map_status("running"), WorkloadState::Running);
        assert_eq!(ProxmoxDriver::map_status("stopped"), WorkloadState::Stopped);
        assert_eq!(ProxmoxDriver::map_status("paused"), WorkloadState::Paused);
    }

    #[test]
    fn token_auth_creates() {
        let driver =
            ProxmoxDriver::with_token("192.168.1.1", "pve1", "user@pam!mytoken", "secret-uuid")
                .unwrap();
        assert_eq!(driver.driver_name(), "proxmox");
    }
}
