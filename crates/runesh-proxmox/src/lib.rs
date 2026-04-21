#![deny(unsafe_code)]
//! Proxmox VE workload driver via REST API.
//!
//! Manages QEMU VMs and LXC containers on Proxmox VE clusters.
//! Authenticates via API token (recommended) or ticket-based auth.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use runesh_workload::{
    CreateSpec, RunResult, TlsConfig, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot,
    WorkloadState, WorkloadType, redact_sensitive, validate_host,
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
    #[allow(dead_code)]
    description: Option<String>,
    #[serde(default)]
    snaptime: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PveTaskStatus {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    exitstatus: Option<String>,
}

fn build_client(tls: &TlsConfig) -> Result<Client, WorkloadError> {
    let mut builder = Client::builder();
    match tls {
        TlsConfig::Verify => {}
        TlsConfig::AcceptInvalidCerts => {
            tracing::warn!(
                "ProxmoxDriver: TLS verification disabled by caller (AcceptInvalidCerts)."
            );
            builder = builder.danger_accept_invalid_certs(true);
        }
        TlsConfig::CustomCa(path) => {
            let pem = std::fs::read(path).map_err(|e| {
                WorkloadError::DriverError(format!("read CA {}: {e}", path.display()))
            })?;
            let cert = reqwest::Certificate::from_pem(&pem)
                .map_err(|e| WorkloadError::DriverError(format!("parse CA: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }
        TlsConfig::PinnedFingerprint(_) => {
            return Err(WorkloadError::NotSupported(
                "pinned fingerprint TLS not implemented for proxmox".into(),
            ));
        }
    }
    builder
        .build()
        .map_err(|e| WorkloadError::DriverError(format!("http client: {e}")))
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
        Self::with_token_tls(host, node, token_id, token_secret, TlsConfig::Verify)
    }

    /// Connect with API token and explicit TLS configuration.
    pub fn with_token_tls(
        host: &str,
        node: &str,
        token_id: &str,
        token_secret: &str,
        tls: TlsConfig,
    ) -> Result<Self, WorkloadError> {
        validate_host(host)?;
        let client = build_client(&tls)?;
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
        Self::with_password_tls(host, node, username, password, TlsConfig::Verify).await
    }

    /// Authenticate with username/password and explicit TLS configuration.
    pub async fn with_password_tls(
        host: &str,
        node: &str,
        username: &str,
        password: &str,
        tls: TlsConfig,
    ) -> Result<Self, WorkloadError> {
        validate_host(host)?;
        let client = build_client(&tls)?;
        let base_url = format!("https://{host}:8006/api2/json");

        let resp = client
            .post(format!("{base_url}/access/ticket"))
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("auth: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let msg = redact_sensitive(&format!("auth failed {status}: {body}"));
            return if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(WorkloadError::auth(msg))
            } else if status.is_server_error() || status.as_u16() == 429 {
                Err(WorkloadError::transient(msg))
            } else {
                Err(WorkloadError::permanent(msg))
            };
        }

        let parsed: PveResponse<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| WorkloadError::transient(format!("auth parse: {e}")))?;

        let ticket = parsed.data["ticket"].as_str().unwrap_or("").to_string();
        let csrf = parsed.data["CSRFPreventionToken"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if ticket.is_empty() || csrf.is_empty() {
            return Err(WorkloadError::auth(
                "Proxmox returned empty ticket/CSRF".to_string(),
            ));
        }

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

    fn classify_status(&self, status: reqwest::StatusCode, body: &str) -> WorkloadError {
        let msg = redact_sensitive(&format!("HTTP {status}: {body}"));
        match status.as_u16() {
            401 | 403 => WorkloadError::auth(msg),
            404 => WorkloadError::NotFound(msg),
            409 => WorkloadError::conflict(msg),
            429 => WorkloadError::transient(msg),
            s if (500..600).contains(&s) => WorkloadError::transient(msg),
            _ => WorkloadError::permanent(msg),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, WorkloadError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .auth_request(self.client.get(&url))
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("GET {path}: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        let parsed: PveResponse<T> = resp
            .json()
            .await
            .map_err(|e| WorkloadError::permanent(format!("parse {path}: {e}")))?;
        Ok(parsed.data)
    }

    async fn post_action(&self, path: &str) -> Result<String, WorkloadError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .auth_request(self.client.post(&url))
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("POST {path}: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        // Proxmox long-running ops return a `data: "UPID:..."` string.
        let body = resp.text().await.unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        Ok(parsed["data"].as_str().unwrap_or("").to_string())
    }

    /// Poll a Proxmox task UPID until it completes or we time out.
    async fn wait_task(
        &self,
        upid: &str,
        timeout: Duration,
        cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        if !upid.starts_with("UPID:") {
            // Not a task response (e.g. synchronous endpoint). Nothing to wait on.
            return Ok(());
        }
        let path = format!("nodes/{}/tasks/{}/status", self.node, upid);
        let url = format!("{}/{}", self.base_url, path);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(c) = &cancel
                && c.is_cancelled()
            {
                return Err(WorkloadError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(WorkloadError::transient(format!(
                    "task {} timed out after {:?}",
                    redact_sensitive(upid),
                    timeout
                )));
            }
            let resp = self
                .auth_request(self.client.get(&url))
                .send()
                .await
                .map_err(|e| WorkloadError::transient(format!("task status: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(self.classify_status(status, &body));
            }
            let parsed: PveResponse<PveTaskStatus> = resp
                .json()
                .await
                .map_err(|e| WorkloadError::transient(format!("task parse: {e}")))?;

            match parsed.data.status.as_deref() {
                Some("stopped") => {
                    if parsed.data.exitstatus.as_deref() == Some("OK") {
                        return Ok(());
                    } else {
                        return Err(WorkloadError::permanent(format!(
                            "task failed: {}",
                            redact_sensitive(
                                parsed.data.exitstatus.as_deref().unwrap_or("unknown")
                            )
                        )));
                    }
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
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

    async fn create(&self, _spec: &CreateSpec) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VM creation requires complex config; use Proxmox UI or API directly".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        let upid = self.post_action(&self.vm_path(id, "/status/start")).await?;
        self.wait_task(&upid, Duration::from_secs(600), None).await
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        let upid = self.post_action(&self.vm_path(id, "/status/stop")).await?;
        self.wait_task(&upid, Duration::from_secs(600), None).await
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        let upid = self
            .post_action(&self.vm_path(id, "/status/reboot"))
            .await?;
        self.wait_task(&upid, Duration::from_secs(600), None).await
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        // DELETE instead of POST for destroy.
        let url = format!("{}/nodes/{}/qemu/{}", self.base_url, self.node, id);
        let resp = self
            .auth_request(self.client.delete(&url))
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("destroy: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        let body = resp.text().await.unwrap_or_default();
        let upid = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["data"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        self.wait_task(&upid, Duration::from_secs(600), None).await
    }

    async fn snapshot(
        &self,
        id: &str,
        name: &str,
        cancel: Option<CancellationToken>,
    ) -> Result<WorkloadSnapshot, WorkloadError> {
        let url = format!("{}/{}", self.base_url, self.vm_path(id, "/snapshot"));
        let resp = self
            .auth_request(self.client.post(&url))
            .form(&[("snapname", name), ("description", "Created by runesh")])
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("snapshot: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        let body = resp.text().await.unwrap_or_default();
        let upid = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["data"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        self.wait_task(&upid, Duration::from_secs(600), cancel)
            .await?;
        Ok(WorkloadSnapshot {
            id: format!("{id}:{name}"),
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
                id: format!("{id}:{}", s.name),
                workload_id: id.to_string(),
                name: s.name,
                created_at: s.snaptime.map(|t| t.to_string()).unwrap_or_default(),
                size_bytes: None,
            })
            .collect())
    }

    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        // snapshot_id format: "vmid:snapname"
        let parts: Vec<&str> = snapshot_id.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(WorkloadError::permanent(
                "snapshot_id format: vmid:snapname".to_string(),
            ));
        }
        let upid = self
            .post_action(&self.vm_path(parts[0], &format!("/snapshot/{}/rollback", parts[1])))
            .await?;
        self.wait_task(&upid, Duration::from_secs(600), cancel)
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
            .map_err(|e| WorkloadError::transient(format!("resize: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
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
