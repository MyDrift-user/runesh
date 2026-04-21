//! Running process snapshot collection.

use sha2::{Digest, Sha256};
use sysinfo::System;

use crate::models::ProcessInfo;

/// Policy for exposing the user identifier on each process record.
///
/// Exposing raw UIDs/SIDs can leak PII across tenant boundaries. Prefer
/// [`PiiPolicy::HashUser`] in anything shipped to operators.
#[derive(Debug, Clone, Default)]
pub enum PiiPolicy {
    /// Expose the raw OS user identifier (UID on Unix, SID on Windows).
    IncludeUser,
    /// Always report `None` for the user field.
    ExcludeUser,
    /// Hash the user identifier with the supplied salt and keep the first
    /// 16 hex characters. This is the default.
    #[default]
    HashUser,
}

/// Options for process collection.
#[derive(Debug, Clone, Default)]
pub struct ProcessOptions {
    pub pii_policy: PiiPolicy,
    /// Salt used when `pii_policy` is `HashUser`. Should be per-tenant and
    /// stable across restarts so values are comparable.
    pub user_hash_salt: Vec<u8>,
}

/// Collect information about all running processes with the default PII
/// policy ([`PiiPolicy::HashUser`], empty salt).
pub fn collect_processes(sys: &System) -> Vec<ProcessInfo> {
    collect_processes_with(sys, &ProcessOptions::default())
}

/// Collect information about all running processes with explicit options.
pub fn collect_processes_with(sys: &System, opts: &ProcessOptions) -> Vec<ProcessInfo> {
    sys.processes()
        .iter()
        .map(|(pid, proc_info)| {
            let status = format!("{:?}", proc_info.status());
            let raw_user = proc_info.user_id().map(|u| u.to_string());
            let user = match &opts.pii_policy {
                PiiPolicy::IncludeUser => raw_user,
                PiiPolicy::ExcludeUser => None,
                PiiPolicy::HashUser => raw_user.map(|u| hash_user(&u, &opts.user_hash_salt)),
            };
            ProcessInfo {
                pid: pid.as_u32(),
                name: proc_info.name().to_string_lossy().to_string(),
                exe_path: proc_info
                    .exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                cmd: proc_info
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect(),
                status,
                cpu_usage: proc_info.cpu_usage(),
                memory_bytes: proc_info.memory(),
                user,
                start_time: proc_info.start_time(),
                parent_pid: proc_info.parent().map(|p| p.as_u32()),
            }
        })
        .collect()
}

fn hash_user(id: &str, salt: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.update(salt);
    let out = h.finalize();
    let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
    hex[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_user_is_stable_and_16_hex() {
        let salt = b"tenant-salt".to_vec();
        let a = hash_user("1000", &salt);
        let b = hash_user("1000", &salt);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        let c = hash_user("1001", &salt);
        assert_ne!(a, c);
    }
}
