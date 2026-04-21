#![deny(unsafe_code)]
//! Append-only hash-chained audit log.
//!
//! Every entry contains the SHA-256 hash of the previous entry,
//! forming a tamper-evident chain. If any entry is modified or
//! deleted, the chain breaks and verification fails.
//!
//! Hashing is domain-separated: each field is prefixed with a big-endian
//! u64 length so that concatenation collisions (e.g. ("ab","c") vs
//! ("a","bc")) cannot produce identical digests. Each entry carries a
//! monotonic `seq` counter that is hashed alongside the content; chain
//! verification rejects any sequence that is not strictly increasing.
//!
//! Hash equality checks use `subtle::ConstantTimeEq` to avoid leaking
//! chain state via timing side channels.
//!
//! Entries can be persisted via the [`AuditSink`] trait. [`InMemoryAuditSink`]
//! keeps everything in RAM (useful for tests and short-lived processes);
//! [`FileAuditSink`] appends JSON Lines to disk, fsyncs after each write,
//! and can reload an existing log on startup.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Errors returned by [`AuditSink`] operations.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("chain broken at entry {0}")]
    ChainBroken(usize),
    #[error("sequence not monotonic at entry {0}")]
    SeqNotMonotonic(usize),
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique entry ID.
    pub id: String,
    /// Monotonically increasing sequence number. Verified by [`verify_entries`]
    /// to catch reordering or deletion attacks that wall-clock timestamps miss.
    pub seq: u64,
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

/// Append a domain-separated, length-prefixed field to the hasher.
///
/// Prefixing each field with its big-endian u64 length prevents concatenation
/// collisions: `("ab","c")` and `("a","bc")` would hash to the same value
/// under naive concatenation but produce different digests here.
fn update_field(hasher: &mut Sha256, name: &str, value: &[u8]) {
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

/// Compute the SHA-256 hash of an entry's content (excluding the hash field itself).
#[allow(clippy::too_many_arguments)]
fn compute_hash(
    id: &str,
    seq: u64,
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
    update_field(&mut hasher, "id", id.as_bytes());
    update_field(&mut hasher, "seq", &seq.to_be_bytes());
    update_field(&mut hasher, "timestamp", timestamp.to_rfc3339().as_bytes());
    update_field(&mut hasher, "prev_hash", prev_hash.as_bytes());
    update_field(&mut hasher, "actor", actor.as_bytes());
    update_field(&mut hasher, "action", action.as_bytes());
    update_field(&mut hasher, "resource", resource.as_bytes());
    update_field(&mut hasher, "details", details.to_string().as_bytes());
    update_field(&mut hasher, "severity", severity.to_string().as_bytes());
    update_field(
        &mut hasher,
        "tenant_id",
        tenant_id.as_deref().unwrap_or("").as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

/// Constant-time hash string comparison. Both strings are lower-hex SHA-256
/// digests, so length mismatches are a short-circuit but the byte comparison
/// itself leaks no information about which byte differed.
fn hash_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).unwrap_u8() == 1
}

/// Verify a slice of entries: chain links, hash contents, and strictly
/// increasing `seq` values. Returns `Ok(())` on success, or the index of
/// the first broken entry.
pub fn verify_entries(entries: &[AuditEntry]) -> Result<(), AuditError> {
    let mut last_seq: Option<u64> = None;
    for (i, entry) in entries.iter().enumerate() {
        let expected_prev = if i == 0 {
            String::new()
        } else {
            entries[i - 1].hash.clone()
        };
        if !hash_eq(&entry.prev_hash, &expected_prev) {
            return Err(AuditError::ChainBroken(i));
        }

        if let Some(prev) = last_seq
            && entry.seq <= prev
        {
            return Err(AuditError::SeqNotMonotonic(i));
        }
        last_seq = Some(entry.seq);

        let computed = compute_hash(
            &entry.id,
            entry.seq,
            &entry.timestamp,
            &entry.prev_hash,
            &entry.actor,
            &entry.action,
            &entry.resource,
            &entry.details,
            entry.severity,
            &entry.tenant_id,
        );
        if !hash_eq(&entry.hash, &computed) {
            return Err(AuditError::ChainBroken(i));
        }
    }
    Ok(())
}

/// Persistent backend for audit entries.
///
/// Implementations must append entries in the order given and return all
/// entries on `iter()` in insertion order.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Append one entry durably. Implementations must ensure the entry is
    /// persisted (e.g. `sync_data`) before returning.
    async fn append(&self, entry: &AuditEntry) -> Result<(), AuditError>;
    /// Return all stored entries in insertion order.
    async fn iter(&self) -> Result<Vec<AuditEntry>, AuditError>;
}

/// In-memory audit sink. Not durable; use for tests or short-lived processes.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditSink {
    inner: Arc<Mutex<Vec<AuditEntry>>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AuditSink for InMemoryAuditSink {
    async fn append(&self, entry: &AuditEntry) -> Result<(), AuditError> {
        self.inner.lock().await.push(entry.clone());
        Ok(())
    }

    async fn iter(&self) -> Result<Vec<AuditEntry>, AuditError> {
        Ok(self.inner.lock().await.clone())
    }
}

/// JSON Lines file-backed audit sink. Each entry is a single line; the file
/// is opened with `append(true)` and `sync_data()` is called after every write.
pub struct FileAuditSink {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl FileAuditSink {
    /// Create a sink for the given path. The file is created if it does not
    /// exist; existing content is preserved and can be loaded via [`iter`].
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl AuditSink for FileAuditSink {
    async fn append(&self, entry: &AuditEntry) -> Result<(), AuditError> {
        let line = serde_json::to_string(entry)?;
        let _guard = self.write_lock.lock().await;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.sync_data().await?;
        Ok(())
    }

    async fn iter(&self) -> Result<Vec<AuditEntry>, AuditError> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file).lines();
        let mut out = Vec::new();
        while let Some(line) = reader.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line)?);
        }
        Ok(out)
    }
}

/// An append-only audit log with hash chain verification.
///
/// The chain is kept in memory for fast verification and filtering; if a
/// [`AuditSink`] is supplied, each append is also persisted durably.
#[derive(Default)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
    next_seq: u64,
    sink: Option<Arc<dyn AuditSink>>,
}

impl fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuditLog")
            .field("entries", &self.entries)
            .field("next_seq", &self.next_seq)
            .field("sink", &self.sink.as_ref().map(|_| "<dyn AuditSink>"))
            .finish()
    }
}

impl AuditLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a persistent sink. Subsequent `append_*` calls will write through
    /// to the sink; existing entries in the log are not flushed.
    pub fn with_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Build a log from entries previously loaded from a sink. Verifies the
    /// chain before returning. The next sequence number continues from the
    /// last loaded entry.
    pub fn from_entries(entries: Vec<AuditEntry>) -> Result<Self, AuditError> {
        verify_entries(&entries)?;
        let next_seq = entries.last().map(|e| e.seq + 1).unwrap_or(0);
        Ok(Self {
            entries,
            next_seq,
            sink: None,
        })
    }

    /// Load a log from a sink, verify, and return it with the sink attached.
    pub async fn load(sink: Arc<dyn AuditSink>) -> Result<Self, AuditError> {
        let entries = sink.iter().await?;
        let mut log = Self::from_entries(entries)?;
        log.sink = Some(sink);
        Ok(log)
    }

    /// Append a new entry to the log. If a sink is attached the entry is
    /// persisted before the in-memory chain is updated, so a crash mid-append
    /// leaves the on-disk log and in-memory state consistent.
    pub async fn append(
        &mut self,
        actor: &str,
        action: &str,
        resource: &str,
        details: serde_json::Value,
        severity: Severity,
        tenant_id: Option<String>,
    ) -> Result<&AuditEntry, AuditError> {
        let entry = self.build_entry(actor, action, resource, details, severity, tenant_id);

        if let Some(sink) = &self.sink {
            sink.append(&entry).await?;
        }

        self.next_seq = entry.seq + 1;
        self.entries.push(entry);
        Ok(self.entries.last().unwrap())
    }

    /// Synchronous append, for callers without a sink or inside non-async contexts.
    /// Panics if a sink is attached; use [`append`] instead.
    pub fn append_in_memory(
        &mut self,
        actor: &str,
        action: &str,
        resource: &str,
        details: serde_json::Value,
        severity: Severity,
        tenant_id: Option<String>,
    ) -> &AuditEntry {
        assert!(
            self.sink.is_none(),
            "append_in_memory called on log with a sink; use append().await"
        );
        let entry = self.build_entry(actor, action, resource, details, severity, tenant_id);
        self.next_seq = entry.seq + 1;
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    fn build_entry(
        &self,
        actor: &str,
        action: &str,
        resource: &str,
        details: serde_json::Value,
        severity: Severity,
        tenant_id: Option<String>,
    ) -> AuditEntry {
        let id = uuid::Uuid::new_v4().to_string();
        let seq = self.next_seq;
        let timestamp = Utc::now();
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_default();

        let hash = compute_hash(
            &id, seq, &timestamp, &prev_hash, actor, action, resource, &details, severity,
            &tenant_id,
        );

        AuditEntry {
            id,
            seq,
            timestamp,
            prev_hash,
            hash,
            actor: actor.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            details,
            severity,
            tenant_id,
        }
    }

    /// Verify the entire chain. Returns the index of the first broken link, if any.
    pub fn verify(&self) -> Result<(), usize> {
        match verify_entries(&self.entries) {
            Ok(()) => Ok(()),
            Err(AuditError::ChainBroken(i)) | Err(AuditError::SeqNotMonotonic(i)) => Err(i),
            Err(_) => Err(0),
        }
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
        Self::from_entries(entries).map_err(|e| format!("{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(log: &mut AuditLog, actor: &str, action: &str, resource: &str) {
        log.append_in_memory(
            actor,
            action,
            resource,
            serde_json::json!({}),
            Severity::Info,
            None,
        );
    }

    #[test]
    fn append_and_verify() {
        let mut log = AuditLog::new();
        append(&mut log, "admin", "login", "session");
        append(&mut log, "admin", "create_user", "user:bob");
        log.append_in_memory(
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
        append(&mut log, "a", "x", "r");
        append(&mut log, "b", "y", "s");

        assert!(log.entries[0].prev_hash.is_empty());
        assert_eq!(log.entries[1].prev_hash, log.entries[0].hash);
    }

    #[test]
    fn seq_is_monotonic() {
        let mut log = AuditLog::new();
        append(&mut log, "a", "x", "r");
        append(&mut log, "b", "y", "s");
        append(&mut log, "c", "z", "t");
        assert_eq!(log.entries[0].seq, 0);
        assert_eq!(log.entries[1].seq, 1);
        assert_eq!(log.entries[2].seq, 2);
    }

    #[test]
    fn reordering_breaks_seq_check() {
        let mut log = AuditLog::new();
        append(&mut log, "a", "x", "r");
        append(&mut log, "b", "y", "s");
        // Swap the seq values but leave hashes alone: chain link should fail
        // either on the seq check or on the hash recompute.
        log.entries.swap(0, 1);
        assert!(log.verify().is_err());
    }

    #[test]
    fn tampered_entry_detected() {
        let mut log = AuditLog::new();
        append(&mut log, "admin", "login", "session");
        append(&mut log, "admin", "action", "resource");

        log.entries[0].actor = "hacker".to_string();
        assert_eq!(log.verify(), Err(0));
    }

    #[test]
    fn broken_chain_detected() {
        let mut log = AuditLog::new();
        append(&mut log, "a", "x", "r");
        append(&mut log, "b", "y", "s");
        log.entries[1].prev_hash = "wrong_hash".to_string();
        assert_eq!(log.verify(), Err(1));
    }

    #[test]
    fn domain_separated_hash_resists_concat_collision() {
        // Naive concatenation would give ("ab","c") and ("a","bc") the same
        // digest. Length prefixes make them distinct.
        let t = Utc::now();
        let a = compute_hash(
            "id",
            0,
            &t,
            "",
            "ab",
            "c",
            "r",
            &serde_json::json!({}),
            Severity::Info,
            &None,
        );
        let b = compute_hash(
            "id",
            0,
            &t,
            "",
            "a",
            "bc",
            "r",
            &serde_json::json!({}),
            Severity::Info,
            &None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn filter_by_actor() {
        let mut log = AuditLog::new();
        append(&mut log, "alice", "login", "session");
        append(&mut log, "bob", "login", "session");
        append(&mut log, "alice", "logout", "session");

        assert_eq!(log.by_actor("alice").len(), 2);
        assert_eq!(log.by_actor("bob").len(), 1);
    }

    #[test]
    fn filter_by_tenant() {
        let mut log = AuditLog::new();
        log.append_in_memory(
            "a",
            "x",
            "r",
            serde_json::json!({}),
            Severity::Info,
            Some("t1".into()),
        );
        log.append_in_memory(
            "b",
            "y",
            "s",
            serde_json::json!({}),
            Severity::Info,
            Some("t2".into()),
        );
        log.append_in_memory(
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
        log.append_in_memory(
            "admin",
            "login",
            "session",
            serde_json::json!({"ip": "1.2.3.4"}),
            Severity::Info,
            None,
        );
        log.append_in_memory(
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
        append(&mut log, "admin", "login", "session");
        let mut json = log.to_json().unwrap();
        json = json.replace("admin", "hacker");
        assert!(AuditLog::from_json(&json).is_err());
    }

    #[test]
    fn severity_filter() {
        let mut log = AuditLog::new();
        log.append_in_memory("a", "x", "r", serde_json::json!({}), Severity::Info, None);
        log.append_in_memory(
            "a",
            "y",
            "r",
            serde_json::json!({}),
            Severity::Critical,
            None,
        );
        log.append_in_memory(
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

    #[tokio::test]
    async fn in_memory_sink_roundtrip() {
        let sink: Arc<dyn AuditSink> = Arc::new(InMemoryAuditSink::new());
        let mut log = AuditLog::new().with_sink(sink.clone());
        log.append(
            "admin",
            "login",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        )
        .await
        .unwrap();
        log.append(
            "admin",
            "logout",
            "session",
            serde_json::json!({}),
            Severity::Info,
            None,
        )
        .await
        .unwrap();

        let reloaded = AuditLog::load(sink).await.unwrap();
        assert_eq!(reloaded.len(), 2);
    }

    #[tokio::test]
    async fn file_sink_durable_across_reopen() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        {
            let sink: Arc<dyn AuditSink> = Arc::new(FileAuditSink::new(&path));
            let mut log = AuditLog::new().with_sink(sink);
            log.append(
                "admin",
                "login",
                "session",
                serde_json::json!({}),
                Severity::Info,
                None,
            )
            .await
            .unwrap();
            log.append(
                "admin",
                "critical",
                "system",
                serde_json::json!({}),
                Severity::Critical,
                None,
            )
            .await
            .unwrap();
        }

        let sink: Arc<dyn AuditSink> = Arc::new(FileAuditSink::new(&path));
        let log = AuditLog::load(sink).await.unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.verify().is_ok());

        let _ = std::fs::remove_file(&path);
    }
}
