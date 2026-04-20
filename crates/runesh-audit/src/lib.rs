#![deny(unsafe_code)]
//! Append-only hash-chained audit log.
//!
//! Every entry contains the SHA-256 hash of the previous entry,
//! forming a tamper-evident chain. If any entry is modified or
//! deleted, the chain breaks and verification fails.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique entry ID.
    pub id: String,
    /// ISO 8601 timestamp.
    pub timestamp: DateTime<Utc>,
    /// Hash of the previous entry (hex). Empty for the first entry.
    pub prev_hash: String,
    /// Hash of this entry (hex), computed from all other fields.
    pub hash: String,
    /// Who performed the action.
    pub actor: String,
    /// What action was performed.
    pub action: String,
    /// What resource was affected.
    pub resource: String,
    /// Additional structured details.
    #[serde(default)]
    pub details: serde_json::Value,
    /// Severity level.
    pub severity: Severity,
    /// Optional tenant scope.
    #[serde(default)]
    pub tenant_id: Option<String>,
}

/// Severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warning => write!(f, "warning"),
            Severity::Critical => write!(f, "critical"),
        }
    }
}

/// Compute the SHA-256 hash of an entry's content (excluding the hash field itself).
fn compute_hash(
    id: &str,
    timestamp: &DateTime<Utc>,
    prev_hash: &str,
    actor: &str,
    action: &str,
    resource: &str,
    details: &serde_json::Value,
    severity: Severity,
    tenant_id: &Option<String>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update(timestamp.to_rfc3339().as_bytes());
    hasher.update(prev_hash.as_bytes());
    hasher.update(actor.as_bytes());
    hasher.update(action.as_bytes());
    hasher.update(resource.as_bytes());
    hasher.update(details.to_string().as_bytes());
    hasher.update(format!("{severity}").as_bytes());
    if let Some(tid) = tenant_id {
        hasher.update(tid.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// An append-only audit log with hash chain verification.
#[derive(Debug, Default)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new entry to the log.
    pub fn append(
        &mut self,
        actor: &str,
        action: &str,
        resource: &str,
        details: serde_json::Value,
        severity: Severity,
        tenant_id: Option<String>,
    ) -> &AuditEntry {
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now();
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();

        let hash = compute_hash(
            &id, &timestamp, &prev_hash, actor, action, resource, &details, severity, &tenant_id,
        );

        self.entries.push(AuditEntry {
            id,
            timestamp,
            prev_hash,
            hash,
            actor: actor.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            details,
            severity,
            tenant_id,
        });

        self.entries.last().unwrap()
    }

    /// Verify the entire chain. Returns the index of the first broken link, if any.
    pub fn verify(&self) -> Result<(), usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            // Check prev_hash links
            let expected_prev = if i == 0 {
                String::new()
            } else {
                self.entries[i - 1].hash.clone()
            };
            if entry.prev_hash != expected_prev {
                return Err(i);
            }

            // Recompute hash and compare
            let computed = compute_hash(
                &entry.id,
                &entry.timestamp,
                &entry.prev_hash,
                &entry.actor,
                &entry.action,
                &entry.resource,
                &entry.details,
                entry.severity,
                &entry.tenant_id,
            );
            if entry.hash != computed {
                return Err(i);
            }
        }
        Ok(())
    }

    /// Get all entries.
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Filter entries by actor.
    pub fn by_actor(&self, actor: &str) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.actor == actor).collect()
    }

    /// Filter entries by action.
    pub fn by_action(&self, action: &str) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.action == action).collect()
    }

    /// Filter entries by tenant.
    pub fn by_tenant(&self, tenant_id: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.tenant_id.as_deref() == Some(tenant_id))
            .collect()
    }

    /// Filter entries by severity.
    pub fn by_severity(&self, severity: Severity) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.severity == severity)
            .collect()
    }

    /// Export the log as JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.entries)
    }

    /// Import entries from JSON. Verifies the chain after import.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let entries: Vec<AuditEntry> =
            serde_json::from_str(json).map_err(|e| format!("parse error: {e}"))?;
        let log = Self { entries };
        log.verify()
            .map_err(|i| format!("chain broken at entry {i}"))?;
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_verify() {
        let mut log = AuditLog::new();
        log.append(
            "admin",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );
        log.append(
            "admin",
            "create_user",
            "user:bob",
            serde_json::json!({"email": "bob@ex.com"}),
            Severity::Info,
            None,
        );
        log.append(
            "admin",
            "delete_user",
            "user:eve",
            serde_json::json!({}),
            Severity::Warning,
            None,
        );

        assert_eq!(log.len(), 3);
        assert!(log.verify().is_ok());
    }

    #[test]
    fn chain_links_correctly() {
        let mut log = AuditLog::new();
        log.append("a", "x", "r", serde_json::json!({}), Severity::Info, None);
        log.append("b", "y", "s", serde_json::json!({}), Severity::Info, None);

        assert!(log.entries[0].prev_hash.is_empty());
        assert_eq!(log.entries[1].prev_hash, log.entries[0].hash);
    }

    #[test]
    fn tampered_entry_detected() {
        let mut log = AuditLog::new();
        log.append(
            "admin",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );
        log.append(
            "admin",
            "action",
            "resource",
            serde_json::json!({}),
            Severity::Info,
            None,
        );

        // Tamper with the first entry
        log.entries[0].actor = "hacker".to_string();

        assert_eq!(log.verify(), Err(0));
    }

    #[test]
    fn broken_chain_detected() {
        let mut log = AuditLog::new();
        log.append("a", "x", "r", serde_json::json!({}), Severity::Info, None);
        log.append("b", "y", "s", serde_json::json!({}), Severity::Info, None);

        // Break the chain link
        log.entries[1].prev_hash = "wrong_hash".to_string();

        assert_eq!(log.verify(), Err(1));
    }

    #[test]
    fn filter_by_actor() {
        let mut log = AuditLog::new();
        log.append(
            "alice",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );
        log.append(
            "bob",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );
        log.append(
            "alice",
            "logout",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );

        assert_eq!(log.by_actor("alice").len(), 2);
        assert_eq!(log.by_actor("bob").len(), 1);
    }

    #[test]
    fn filter_by_tenant() {
        let mut log = AuditLog::new();
        log.append(
            "a",
            "x",
            "r",
            serde_json::json!({}),
            Severity::Info,
            Some("t1".into()),
        );
        log.append(
            "b",
            "y",
            "s",
            serde_json::json!({}),
            Severity::Info,
            Some("t2".into()),
        );
        log.append(
            "c",
            "z",
            "t",
            serde_json::json!({}),
            Severity::Info,
            Some("t1".into()),
        );

        assert_eq!(log.by_tenant("t1").len(), 2);
        assert_eq!(log.by_tenant("t2").len(), 1);
    }

    #[test]
    fn json_roundtrip() {
        let mut log = AuditLog::new();
        log.append(
            "admin",
            "login",
            "session",
            serde_json::json!({"ip": "1.2.3.4"}),
            Severity::Info,
            None,
        );
        log.append(
            "admin",
            "critical_action",
            "system",
            serde_json::json!({}),
            Severity::Critical,
            None,
        );

        let json = log.to_json().unwrap();
        let restored = AuditLog::from_json(&json).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.verify().is_ok());
    }

    #[test]
    fn tampered_json_rejected() {
        let mut log = AuditLog::new();
        log.append(
            "admin",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        );

        let mut json = log.to_json().unwrap();
        json = json.replace("admin", "hacker");

        assert!(AuditLog::from_json(&json).is_err());
    }

    #[test]
    fn severity_filter() {
        let mut log = AuditLog::new();
        log.append("a", "x", "r", serde_json::json!({}), Severity::Info, None);
        log.append(
            "a",
            "y",
            "r",
            serde_json::json!({}),
            Severity::Critical,
            None,
        );
        log.append(
            "a",
            "z",
            "r",
            serde_json::json!({}),
            Severity::Warning,
            None,
        );

        assert_eq!(log.by_severity(Severity::Critical).len(), 1);
        assert_eq!(log.by_severity(Severity::Info).len(), 1);
    }
}
