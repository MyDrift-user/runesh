#![deny(unsafe_code)]
//! Uniform workload driver trait for VMs, containers, and Kubernetes.
//!
//! This crate defines the trait. Actual driver implementations live in
//! separate crates: runesh-docker, runesh-k8s, runesh-hyperv,
//! runesh-proxmox, runesh-vmware.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

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

/// Specification for creating a workload.
///
/// `idempotency_key` must be unique per logical create. Drivers should persist
/// it as a tag/label/custom attribute on the target (Docker label,
/// vCenter custom attribute, Proxmox tag) so a retried create detects and
/// returns the existing workload instead of creating a duplicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSpec {
    /// Driver-specific body (image, config, resources).
    pub spec: serde_json::Value,
    /// Stable key used for idempotent creates. Required.
    pub idempotency_key: String,
    /// Tags propagated to the workload for discovery and scoping.
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

/// TLS verification mode for HTTPS clients.
#[derive(Debug, Clone, Default)]
pub enum TlsConfig {
    /// Standard CA-based verification. Default.
    #[default]
    Verify,
    /// Accept any certificate. Dev or self-signed on-prem only.
    /// Must be set explicitly by the caller.
    AcceptInvalidCerts,
    /// Verify against a custom CA bundle (PEM file).
    CustomCa(PathBuf),
    /// Verify against a pinned SHA-256 fingerprint of the leaf cert.
    PinnedFingerprint(Vec<u8>),
}

/// Uniform driver trait for workload management.
#[async_trait]
pub trait WorkloadDriver: Send + Sync {
    fn driver_name(&self) -> &str;

    async fn list(&self) -> Result<Vec<Workload>, WorkloadError>;
    async fn get(&self, id: &str) -> Result<Workload, WorkloadError>;
    async fn create(&self, spec: &CreateSpec) -> Result<Workload, WorkloadError>;
    async fn start(&self, id: &str) -> Result<(), WorkloadError>;
    async fn stop(&self, id: &str) -> Result<(), WorkloadError>;
    async fn restart(&self, id: &str) -> Result<(), WorkloadError>;
    async fn destroy(&self, id: &str) -> Result<(), WorkloadError>;

    async fn snapshot(
        &self,
        id: &str,
        name: &str,
        cancel: Option<CancellationToken>,
    ) -> Result<WorkloadSnapshot, WorkloadError>;
    async fn list_snapshots(&self, id: &str) -> Result<Vec<WorkloadSnapshot>, WorkloadError>;
    async fn restore_snapshot(
        &self,
        snapshot_id: &str,
        cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError>;

    async fn run_command(&self, id: &str, command: &[&str]) -> Result<RunResult, WorkloadError>;
    async fn logs(&self, id: &str, lines: u32) -> Result<String, WorkloadError>;

    async fn resize(
        &self,
        id: &str,
        cpu: Option<u32>,
        memory_mb: Option<u64>,
    ) -> Result<(), WorkloadError>;

    /// Optional: migrate workload to another host. Long-running, cancellable.
    async fn migrate(
        &self,
        _id: &str,
        _target_host: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        Err(WorkloadError::NotSupported("migrate".into()))
    }

    /// Optional: back up workload data. Long-running, cancellable.
    async fn backup(
        &self,
        _id: &str,
        _cancel: Option<CancellationToken>,
    ) -> Result<(), WorkloadError> {
        Err(WorkloadError::NotSupported("backup".into()))
    }
}

/// Classification of an operation failure. Used by callers to decide whether
/// to retry, back off, escalate, or surface to the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network blip, timeout, upstream 5xx, rate limit. Safe to retry with backoff.
    Transient,
    /// Permanent config/logic error. Do not retry.
    Permanent,
    /// Authentication or authorization failure. Refresh credentials.
    Auth,
    /// Target does not exist.
    NotFound,
    /// Resource conflict (duplicate, busy, state mismatch).
    Conflict,
}

impl ErrorKind {
    pub fn is_transient(&self) -> bool {
        matches!(self, ErrorKind::Transient)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkloadError {
    #[error("workload not found: {0}")]
    NotFound(String),
    #[error("operation failed [{kind:?}]: {message}")]
    OperationFailed { kind: ErrorKind, message: String },
    #[error("driver error: {0}")]
    DriverError(String),
    #[error("not supported: {0}")]
    NotSupported(String),
    #[error("operation cancelled")]
    Cancelled,
}

impl WorkloadError {
    pub fn transient(msg: impl Into<String>) -> Self {
        Self::OperationFailed {
            kind: ErrorKind::Transient,
            message: msg.into(),
        }
    }
    pub fn permanent(msg: impl Into<String>) -> Self {
        Self::OperationFailed {
            kind: ErrorKind::Permanent,
            message: msg.into(),
        }
    }
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::OperationFailed {
            kind: ErrorKind::Auth,
            message: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::OperationFailed {
            kind: ErrorKind::Conflict,
            message: msg.into(),
        }
    }

    /// True if the error is a transient failure and retrying is reasonable.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            WorkloadError::OperationFailed {
                kind: ErrorKind::Transient,
                ..
            }
        )
    }
}

/// Redact secret-shaped substrings from an error or response body before
/// storing or logging. Truncates to 4 KiB to avoid runaway messages.
pub fn redact_sensitive(s: &str) -> String {
    const MAX: usize = 4096;
    let truncated = if s.len() > MAX { &s[..MAX] } else { s };

    let mut out = String::with_capacity(truncated.len());
    let lower = truncated.to_ascii_lowercase();
    let bytes = truncated.as_bytes();
    let lower_bytes = lower.as_bytes();
    let mut i = 0;

    // Known secret key prefixes to scrub (case-insensitive).
    let prefixes: &[(&[u8], usize)] = &[
        (b"password=", 9),
        (b"passwd=", 7),
        (b"token=", 6),
        (b"secret=", 7),
        (b"api_key=", 8),
        (b"apikey=", 7),
        (b"authorization: bearer ", 22),
        (b"authorization: basic ", 21),
        (b"bearer ", 7),
        (b"csrfpreventiontoken=", 20),
        (b"pveauthcookie=", 14),
        (b"pveapitoken=", 12),
    ];

    while i < bytes.len() {
        let mut matched = false;
        for (pfx, plen) in prefixes {
            let end = i + plen;
            if end <= lower_bytes.len() && &lower_bytes[i..end] == *pfx {
                // Copy the literal prefix from original (preserves case).
                out.push_str(&truncated[i..end]);
                out.push_str("<redacted>");
                // Advance past the value until whitespace, comma, ;, or &.
                i = end;
                while i < bytes.len()
                    && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b',' | b';' | b'&')
                {
                    i += 1;
                }
                matched = true;
                break;
            }
        }
        if !matched {
            // Safe push of one byte via char boundary.
            let ch = truncated[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    if s.len() > MAX {
        out.push_str("...<truncated>");
    }
    out
}

/// Validate that `host` is a non-empty hostname or IP and not an IMDS endpoint.
/// Drivers must call this at construction to block SSRF pivots.
pub fn validate_host(host: &str) -> Result<(), WorkloadError> {
    if host.is_empty() {
        return Err(WorkloadError::permanent("host is empty"));
    }
    if host.contains('/') || host.contains(' ') {
        return Err(WorkloadError::permanent(format!(
            "invalid host: {}",
            redact_sensitive(host)
        )));
    }
    // Reject AWS/GCP/Azure IMDS endpoints outright.
    let banned = [
        "169.254.169.254",
        "fd00:ec2::254",
        "metadata.google.internal",
        "metadata.goog",
    ];
    let h = host.to_ascii_lowercase();
    if banned.iter().any(|b| h == *b) {
        return Err(WorkloadError::permanent(
            "host resolves to a banned metadata endpoint".to_string(),
        ));
    }
    // Must be a parseable Host.
    url::Host::parse(host)
        .map(|_| ())
        .map_err(|e| WorkloadError::permanent(format!("invalid host: {e}")))
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

    #[test]
    fn error_kind_is_transient() {
        let t = WorkloadError::transient("timeout");
        let p = WorkloadError::permanent("bad spec");
        let a = WorkloadError::auth("401");
        let c = WorkloadError::conflict("busy");
        assert!(t.is_transient());
        assert!(!p.is_transient());
        assert!(!a.is_transient());
        assert!(!c.is_transient());
        assert!(ErrorKind::Transient.is_transient());
        assert!(!ErrorKind::Permanent.is_transient());
        assert!(!ErrorKind::Auth.is_transient());
        assert!(!ErrorKind::NotFound.is_transient());
        assert!(!ErrorKind::Conflict.is_transient());
    }

    #[test]
    fn redact_sensitive_scrubs_known_patterns() {
        let s = "POST /api?password=hunter2&user=alice token=abc123";
        let r = redact_sensitive(s);
        assert!(!r.contains("hunter2"), "r={r}");
        assert!(!r.contains("abc123"), "r={r}");
        assert!(r.contains("password=<redacted>"));
        assert!(r.contains("token=<redacted>"));
        assert!(r.contains("user=alice"));
    }

    #[test]
    fn redact_sensitive_truncates() {
        let s = "a".repeat(10_000);
        let r = redact_sensitive(&s);
        assert!(r.len() <= 4096 + "...<truncated>".len());
        assert!(r.ends_with("...<truncated>"));
    }

    #[test]
    fn validate_host_accepts_valid() {
        validate_host("192.168.1.1").unwrap();
        validate_host("example.com").unwrap();
        validate_host("vcenter.corp.local").unwrap();
    }

    #[test]
    fn validate_host_rejects_imds() {
        assert!(validate_host("169.254.169.254").is_err());
        assert!(validate_host("metadata.google.internal").is_err());
    }

    #[test]
    fn validate_host_rejects_junk() {
        assert!(validate_host("").is_err());
        assert!(validate_host("has space").is_err());
        assert!(validate_host("has/slash").is_err());
    }

    #[test]
    fn create_spec_round_trip() {
        let mut tags = HashMap::new();
        tags.insert("owner".into(), "alice".into());
        let spec = CreateSpec {
            spec: serde_json::json!({"image": "nginx"}),
            idempotency_key: "abc-123".into(),
            tags,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: CreateSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.idempotency_key, "abc-123");
        assert_eq!(back.tags.get("owner").unwrap(), "alice");
    }
}
