#![deny(unsafe_code)]
//! VMware vCenter/ESXi workload driver via REST API.

use async_trait::async_trait;
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use runesh_workload::{
    CreateSpec, RunResult, TlsConfig, Workload, WorkloadDriver, WorkloadError, WorkloadSnapshot,
    WorkloadState, WorkloadType, redact_sensitive, validate_host,
};

const SESSION_LIFETIME_SECS: i64 = 30 * 60; // vCenter sessions default to ~30 min idle.
const SESSION_REFRESH_BEFORE_SECS: i64 = 60;

#[derive(Clone)]
struct Session {
    id: String,
    /// Absolute epoch seconds at which the session expires.
    expires_at_epoch_secs: i64,
}

pub struct VmwareDriver {
    client: Client,
    base_url: String,
    username: String,
    password: String,
    session: Arc<Mutex<Option<Session>>>,
}

#[derive(Debug, Deserialize)]
struct VcVm {
    vm: String,
    name: String,
    power_state: String,
    #[serde(default)]
    cpu_count: Option<u32>,
    /// vCenter 7 uses `memory_size_MiB`; vCenter 8 canonically uses
    /// `memory_size_mib`. Accept either.
    #[serde(default, rename = "memory_size_MiB", alias = "memory_size_mib")]
    memory_size_mib: Option<u64>,
}

fn build_client(tls: &TlsConfig) -> Result<Client, WorkloadError> {
    let mut builder = Client::builder();
    match tls {
        TlsConfig::Verify => {}
        TlsConfig::AcceptInvalidCerts => {
            tracing::warn!(
                "VmwareDriver: TLS verification disabled by caller (AcceptInvalidCerts)."
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
                "pinned fingerprint TLS not implemented for vmware".into(),
            ));
        }
    }
    builder
        .build()
        .map_err(|e| WorkloadError::DriverError(format!("http: {e}")))
}

impl VmwareDriver {
    pub async fn connect(
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, WorkloadError> {
        Self::connect_tls(host, username, password, TlsConfig::Verify).await
    }

    pub async fn connect_tls(
        host: &str,
        username: &str,
        password: &str,
        tls: TlsConfig,
    ) -> Result<Self, WorkloadError> {
        validate_host(host)?;
        let client = build_client(&tls)?;
        let base_url = format!("https://{host}");

        let driver = Self {
            client,
            base_url,
            username: username.to_string(),
            password: password.to_string(),
            session: Arc::new(Mutex::new(None)),
        };
        driver.authenticate().await?;
        Ok(driver)
    }

    async fn authenticate(&self) -> Result<String, WorkloadError> {
        let resp = self
            .client
            .post(format!("{}/api/session", self.base_url))
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("auth: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let msg = redact_sensitive(&format!("auth {status}: {body}"));
            return if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(WorkloadError::auth(msg))
            } else if status.is_server_error() || status.as_u16() == 429 {
                Err(WorkloadError::transient(msg))
            } else {
                Err(WorkloadError::permanent(msg))
            };
        }

        let id = resp
            .text()
            .await
            .map_err(|e| WorkloadError::transient(format!("session: {e}")))?
            .trim()
            .trim_matches('"')
            .to_string();

        if id.is_empty() {
            return Err(WorkloadError::auth("empty session id".to_string()));
        }

        let session = Session {
            id: id.clone(),
            expires_at_epoch_secs: Utc::now().timestamp() + SESSION_LIFETIME_SECS,
        };
        *self.session.lock().await = Some(session);
        Ok(id)
    }

    /// Return a valid session id, refreshing if we're close to expiry.
    async fn session_id(&self) -> Result<String, WorkloadError> {
        {
            let guard = self.session.lock().await;
            if let Some(s) = guard.as_ref()
                && Utc::now().timestamp() + SESSION_REFRESH_BEFORE_SECS < s.expires_at_epoch_secs
            {
                return Ok(s.id.clone());
            }
        }
        // Either missing or near-expiry: re-auth.
        self.authenticate().await
    }

    fn map_power(state: &str) -> WorkloadState {
        match state {
            "POWERED_ON" => WorkloadState::Running,
            "POWERED_OFF" => WorkloadState::Stopped,
            "SUSPENDED" => WorkloadState::Paused,
            _ => WorkloadState::Unknown,
        }
    }

    /// Attach the current session header.
    async fn auth(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, WorkloadError> {
        let id = self.session_id().await?;
        Ok(req.header("vmware-api-session-id", id))
    }

    fn classify_status(&self, status: StatusCode, body: &str) -> WorkloadError {
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

    /// Execute a request builder (clonable), retrying once on 401 after a
    /// forced re-auth.
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, WorkloadError> {
        let first = self
            .auth(build())
            .await?
            .send()
            .await
            .map_err(|e| WorkloadError::transient(format!("send: {e}")))?;
        if first.status() == StatusCode::UNAUTHORIZED {
            // Force re-auth and retry once.
            *self.session.lock().await = None;
            let _ = self.session_id().await?;
            let second = self
                .auth(build())
                .await?
                .send()
                .await
                .map_err(|e| WorkloadError::transient(format!("retry send: {e}")))?;
            return Ok(second);
        }
        Ok(first)
    }
}

impl Drop for VmwareDriver {
    fn drop(&mut self) {
        // Best-effort logout. We cannot block here, so if we're inside a
        // tokio runtime we spawn a detached task; otherwise we skip.
        let session = self.session.clone();
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let maybe = { session.lock().await.clone() };
                if let Some(s) = maybe {
                    let _ = client
                        .delete(format!("{base_url}/rest/com/vmware/cis/session"))
                        .header("vmware-api-session-id", s.id)
                        .send()
                        .await;
                }
            });
        }
    }
}

#[async_trait]
impl WorkloadDriver for VmwareDriver {
    fn driver_name(&self) -> &str {
        "vmware"
    }

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError> {
        let url = format!("{}/api/vcenter/vm", self.base_url);
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        let vms: Vec<VcVm> = resp
            .json()
            .await
            .map_err(|e| WorkloadError::permanent(format!("parse: {e}")))?;
        Ok(vms
            .into_iter()
            .map(|vm| Workload {
                id: vm.vm.clone(),
                name: vm.name,
                workload_type: WorkloadType::Vm,
                state: Self::map_power(&vm.power_state),
                cpu_cores: vm.cpu_count,
                memory_mb: vm.memory_size_mib,
                disk_gb: None,
                image: None,
                host: None,
                ips: vec![],
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Workload, WorkloadError> {
        let url = format!("{}/api/vcenter/vm/{id}", self.base_url);
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        let vm: VcVm = resp
            .json()
            .await
            .map_err(|e| WorkloadError::permanent(format!("parse: {e}")))?;
        Ok(Workload {
            id: vm.vm,
            name: vm.name,
            workload_type: WorkloadType::Vm,
            state: Self::map_power(&vm.power_state),
            cpu_cores: vm.cpu_count,
            memory_mb: vm.memory_size_mib,
            disk_gb: None,
            image: None,
            host: None,
            ips: vec![],
        })
    }

    async fn create(&self, _: &CreateSpec) -> Result<Workload, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "VM creation requires complex config".into(),
        ))
    }

    async fn start(&self, id: &str) -> Result<(), WorkloadError> {
        let url = format!("{}/api/vcenter/vm/{id}/power/start", self.base_url);
        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        Ok(())
    }

    async fn stop(&self, id: &str) -> Result<(), WorkloadError> {
        let url = format!("{}/api/vcenter/vm/{id}/power/stop", self.base_url);
        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        Ok(())
    }

    async fn restart(&self, id: &str) -> Result<(), WorkloadError> {
        let url = format!("{}/api/vcenter/vm/{id}/power/reset", self.base_url);
        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), WorkloadError> {
        let url = format!("{}/api/vcenter/vm/{id}", self.base_url);
        let resp = self.send_with_retry(|| self.client.delete(&url)).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(self.classify_status(status, &body));
        }
        Ok(())
    }

    async fn snapshot(
        &self,
        _: &str,
        _: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<WorkloadSnapshot, WorkloadError> {
        Err(WorkloadError::NotSupported(
            "use vSphere API for snapshots".into(),
        ))
    }
    async fn list_snapshots(&self, _: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError> {
        Ok(vec![])
    }
    async fn restore_snapshot(
        &self,
        _: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
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
    async fn resize(
        &self,
        id: &str,
        cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError> {
        if cpu.is_none() && memory_mb.is_none() {
            return Ok(());
        }

        // vCenter requires the VM to be powered off for CPU / memory edits.
        // We surface vCenter's own error when it isn't, rather than trying
        // to power it off ourselves; turning a VM off is a separate
        // decision that belongs to the caller.
        if let Some(count) = cpu {
            let url = format!("{}/api/vcenter/vm/{id}/hardware/cpu", self.base_url);
            let body = serde_json::json!({ "count": count });
            let resp = self
                .send_with_retry(|| self.client.patch(&url).json(&body))
                .await?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(self.classify_status(status, &body));
            }
        }

        if let Some(mib) = memory_mb {
            let url = format!("{}/api/vcenter/vm/{id}/hardware/memory", self.base_url);
            // vSphere 7 used `size_MiB`, vSphere 8 accepts both spellings but
            // the canonical JSON key is `size_mib`. Send both so neither
            // version 404s us, vCenter ignores the unknown one.
            let body = serde_json::json!({ "size_mib": mib, "size_MiB": mib });
            let resp = self
                .send_with_retry(|| self.client.patch(&url).json(&body))
                .await?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(self.classify_status(status, &body));
            }
        }

        Ok(())
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

    #[test]
    fn vcvm_accepts_both_memory_keys() {
        let v7: VcVm = serde_json::from_str(
            r#"{"vm":"vm-1","name":"a","power_state":"POWERED_ON","memory_size_MiB":4096}"#,
        )
        .unwrap();
        assert_eq!(v7.memory_size_mib, Some(4096));

        let v8: VcVm = serde_json::from_str(
            r#"{"vm":"vm-2","name":"b","power_state":"POWERED_OFF","memory_size_mib":2048}"#,
        )
        .unwrap();
        assert_eq!(v8.memory_size_mib, Some(2048));
    }
}
