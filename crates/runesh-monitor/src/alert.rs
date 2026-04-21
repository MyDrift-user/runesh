//! Alert state machine with flap prevention.
//!
//! State transitions:
//!   OK -> Pending (first failure)
//!   Pending -> Firing (threshold consecutive failures)
//!   Pending -> OK (success before threshold)
//!   Firing -> Resolved (threshold consecutive successes)
//!   Resolved -> OK (after notification sent)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::check::{CheckResult, CheckStatus};

/// Alert state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertState {
    Ok,
    Pending,
    Firing,
    Resolved,
}

/// A tracked alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub check_id: String,
    pub check_name: String,
    pub state: AlertState,
    pub message: String,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub last_check: Option<DateTime<Utc>>,
    pub fired_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Event emitted when an alert state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub check_id: String,
    pub check_name: String,
    pub from_state: AlertState,
    pub to_state: AlertState,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

impl Alert {
    pub fn new(
        check_id: &str,
        check_name: &str,
        failure_threshold: u32,
        recovery_threshold: u32,
    ) -> Self {
        Self {
            check_id: check_id.to_string(),
            check_name: check_name.to_string(),
            state: AlertState::Ok,
            message: String::new(),
            consecutive_failures: 0,
            consecutive_successes: 0,
            failure_threshold,
            recovery_threshold,
            last_check: None,
            fired_at: None,
            resolved_at: None,
        }
    }

    /// Process a check result and return a state change event if one occurred.
    pub fn process(&mut self, result: &CheckResult) -> Option<AlertEvent> {
        self.last_check = Some(result.timestamp);
        let prev_state = self.state;

        match result.status {
            CheckStatus::Ok => {
                self.consecutive_failures = 0;
                self.consecutive_successes += 1;

                match self.state {
                    AlertState::Pending => {
                        self.state = AlertState::Ok;
                        self.message.clear();
                    }
                    AlertState::Firing => {
                        if self.consecutive_successes >= self.recovery_threshold {
                            self.state = AlertState::Resolved;
                            self.resolved_at = Some(Utc::now());
                        }
                    }
                    AlertState::Resolved => {
                        self.state = AlertState::Ok;
                    }
                    AlertState::Ok => {}
                }
            }
            CheckStatus::Critical | CheckStatus::Warning => {
                self.consecutive_successes = 0;
                self.consecutive_failures += 1;
                self.message = result.message.clone();

                match self.state {
                    AlertState::Ok => {
                        self.state = AlertState::Pending;
                    }
                    AlertState::Pending => {
                        if self.consecutive_failures >= self.failure_threshold {
                            self.state = AlertState::Firing;
                            self.fired_at = Some(Utc::now());
                        }
                    }
                    AlertState::Firing | AlertState::Resolved => {}
                }
            }
            CheckStatus::Unknown => {}
        }

        if self.state != prev_state {
            Some(AlertEvent {
                check_id: self.check_id.clone(),
                check_name: self.check_name.clone(),
                from_state: prev_state,
                to_state: self.state,
                message: self.message.clone(),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }
}

/// Manages alerts for multiple checks.
#[derive(Debug, Default)]
pub struct AlertManager {
    alerts: HashMap<String, Alert>,
}

/// Persistable snapshot of all tracked alerts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertSnapshot {
    pub alerts: HashMap<String, Alert>,
}

impl AlertManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a check for alert tracking.
    ///
    /// If an alert with the same `check_id` already exists (for example, loaded
    /// from an `AlertStore`), this preserves its state and only updates
    /// thresholds and display name. This is important for persistence: on
    /// startup we load persisted state then re-register every configured check.
    pub fn register(
        &mut self,
        check_id: &str,
        check_name: &str,
        failure_threshold: u32,
        recovery_threshold: u32,
    ) {
        if let Some(existing) = self.alerts.get_mut(check_id) {
            existing.check_name = check_name.to_string();
            existing.failure_threshold = failure_threshold;
            existing.recovery_threshold = recovery_threshold;
        } else {
            self.alerts.insert(
                check_id.to_string(),
                Alert::new(check_id, check_name, failure_threshold, recovery_threshold),
            );
        }
    }

    /// Process a check result. Returns an alert event if state changed.
    pub fn process(&mut self, result: &CheckResult) -> Option<AlertEvent> {
        self.alerts
            .get_mut(&result.check_id)
            .and_then(|alert| alert.process(result))
    }

    /// Get all currently firing alerts.
    pub fn firing(&self) -> Vec<&Alert> {
        self.alerts
            .values()
            .filter(|a| a.state == AlertState::Firing)
            .collect()
    }

    /// Get all alerts.
    pub fn all(&self) -> Vec<&Alert> {
        self.alerts.values().collect()
    }

    /// Get a specific alert by check ID.
    pub fn get(&self, check_id: &str) -> Option<&Alert> {
        self.alerts.get(check_id)
    }

    /// Build a serializable snapshot of the entire alert table.
    pub fn snapshot(&self) -> AlertSnapshot {
        AlertSnapshot {
            alerts: self.alerts.clone(),
        }
    }

    /// Replace in-memory state with a loaded snapshot.
    pub fn restore_from(&mut self, snap: AlertSnapshot) {
        self.alerts = snap.alerts;
    }
}

/// Persistent alert-state backing store.
///
/// Implementations must serialize a snapshot and return it on `load`. The
/// scheduler calls `save` after every check cycle, and `load` once on startup.
#[async_trait]
pub trait AlertStore: Send + Sync + std::fmt::Debug {
    async fn save(&self, snap: &AlertSnapshot) -> Result<(), AlertStoreError>;
    async fn load(&self) -> Result<AlertSnapshot, AlertStoreError>;
}

/// Errors returned by `AlertStore` implementations.
#[derive(Debug, thiserror::Error)]
pub enum AlertStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// In-memory alert store -- test-only.
#[derive(Debug, Default)]
pub struct InMemoryAlertStore {
    inner: tokio::sync::Mutex<AlertSnapshot>,
}

#[async_trait]
impl AlertStore for InMemoryAlertStore {
    async fn save(&self, snap: &AlertSnapshot) -> Result<(), AlertStoreError> {
        *self.inner.lock().await = snap.clone();
        Ok(())
    }
    async fn load(&self) -> Result<AlertSnapshot, AlertStoreError> {
        Ok(self.inner.lock().await.clone())
    }
}

/// JSON-on-disk alert store with atomic-rename writes.
#[derive(Debug)]
pub struct FileAlertStore {
    path: PathBuf,
}

impl FileAlertStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

#[async_trait]
impl AlertStore for FileAlertStore {
    async fn save(&self, snap: &AlertSnapshot) -> Result<(), AlertStoreError> {
        let path = self.path.clone();
        let data = serde_json::to_vec_pretty(snap)?;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Write to a sibling tmp file and atomically rename.
            let mut tmp = path.clone();
            let name = tmp
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("alerts")
                .to_string();
            tmp.set_file_name(format!(".{name}.tmp"));
            std::fs::write(&tmp, &data)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;
        Ok(())
    }

    async fn load(&self) -> Result<AlertSnapshot, AlertStoreError> {
        let path = self.path.clone();
        let bytes = tokio::task::spawn_blocking(move || -> std::io::Result<Option<Vec<u8>>> {
            match std::fs::read(&path) {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;
        match bytes {
            None => Ok(AlertSnapshot::default()),
            Some(b) => Ok(serde_json::from_slice(&b)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_result(check_id: &str) -> CheckResult {
        CheckResult {
            check_id: check_id.into(),
            status: CheckStatus::Ok,
            message: "OK".into(),
            duration_ms: 10,
            timestamp: Utc::now(),
        }
    }

    fn fail_result(check_id: &str) -> CheckResult {
        CheckResult {
            check_id: check_id.into(),
            status: CheckStatus::Critical,
            message: "connection refused".into(),
            duration_ms: 10,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn ok_to_pending_on_failure() {
        let mut alert = Alert::new("c1", "Check 1", 3, 2);
        let event = alert.process(&fail_result("c1"));
        assert!(event.is_some());
        assert_eq!(alert.state, AlertState::Pending);
    }

    #[test]
    fn pending_back_to_ok_on_success() {
        let mut alert = Alert::new("c1", "Check 1", 3, 2);
        alert.process(&fail_result("c1"));
        assert_eq!(alert.state, AlertState::Pending);

        let event = alert.process(&ok_result("c1"));
        assert!(event.is_some());
        assert_eq!(alert.state, AlertState::Ok);
    }

    #[test]
    fn pending_to_firing_after_threshold() {
        let mut alert = Alert::new("c1", "Check 1", 3, 2);
        alert.process(&fail_result("c1")); // -> Pending
        alert.process(&fail_result("c1")); // still Pending
        let event = alert.process(&fail_result("c1")); // -> Firing (3rd failure)

        assert!(event.is_some());
        assert_eq!(event.unwrap().to_state, AlertState::Firing);
        assert_eq!(alert.state, AlertState::Firing);
    }

    #[test]
    fn firing_to_resolved_after_recovery() {
        let mut alert = Alert::new("c1", "Check 1", 2, 2);
        alert.process(&fail_result("c1"));
        alert.process(&fail_result("c1")); // -> Firing

        alert.process(&ok_result("c1")); // 1 success, not enough
        assert_eq!(alert.state, AlertState::Firing);

        let event = alert.process(&ok_result("c1")); // 2 successes -> Resolved
        assert!(event.is_some());
        assert_eq!(alert.state, AlertState::Resolved);
    }

    #[test]
    fn resolved_to_ok() {
        let mut alert = Alert::new("c1", "Check 1", 1, 1);
        alert.process(&fail_result("c1")); // -> Firing
        alert.process(&ok_result("c1")); // -> Resolved
        alert.process(&ok_result("c1")); // -> Ok
        assert_eq!(alert.state, AlertState::Ok);
    }

    #[test]
    fn alert_manager() {
        let mut mgr = AlertManager::new();
        mgr.register("c1", "HTTP Check", 2, 1);
        mgr.register("c2", "TCP Check", 2, 1);

        mgr.process(&fail_result("c1"));
        mgr.process(&fail_result("c1"));
        assert_eq!(mgr.firing().len(), 1);

        mgr.process(&ok_result("c1"));
        assert_eq!(mgr.firing().len(), 0);
    }

    #[test]
    fn no_event_when_state_unchanged() {
        let mut alert = Alert::new("c1", "Check 1", 3, 2);
        alert.process(&fail_result("c1")); // -> Pending (event)
        let event = alert.process(&fail_result("c1")); // still Pending (no event)
        assert!(event.is_none());
    }
}
