//! Audit logging for remote CLI and file operations.
//!
//! Logs all security-relevant events for compliance and forensics.

use serde::Serialize;

/// Audit log entry.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub event: String,
    pub user: Option<String>,
    pub session_id: Option<String>,
    pub details: serde_json::Value,
}

/// Audit logger that records all remote operations.
pub struct AuditLogger {
    /// If true, also write entries to a file.
    log_to_file: bool,
    file_path: Option<std::path::PathBuf>,
}

impl AuditLogger {
    /// Create a new audit logger that only logs via tracing.
    pub fn new() -> Self {
        Self {
            log_to_file: false,
            file_path: None,
        }
    }

    /// Create an audit logger that also writes to a file.
    pub fn with_file(path: std::path::PathBuf) -> Self {
        Self {
            log_to_file: true,
            file_path: Some(path),
        }
    }

    /// Log a generic audit event.
    pub async fn log(&self, entry: &AuditEntry) {
        tracing::info!(
            event = %entry.event,
            user = ?entry.user,
            session_id = ?entry.session_id,
            details = %entry.details,
            "AUDIT"
        );

        if self.log_to_file {
            if let Some(ref path) = self.file_path {
                if let Ok(json) = serde_json::to_string(entry) {
                    use tokio::io::AsyncWriteExt;
                    if let Ok(mut file) = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .await
                    {
                        let _ = file.write_all(format!("{json}\n").as_bytes()).await;
                    }
                }
            }
        }
    }

    /// Log a CLI session open event.
    pub async fn log_session_open(&self, session_id: &str, shell: &str, user: Option<&str>) {
        self.log(&AuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event: "session_open".into(),
            user: user.map(String::from),
            session_id: Some(session_id.into()),
            details: serde_json::json!({ "shell": shell }),
        })
        .await;
    }

    /// Log a CLI session close event.
    pub async fn log_session_close(
        &self,
        session_id: &str,
        exit_code: Option<u32>,
        user: Option<&str>,
    ) {
        self.log(&AuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event: "session_close".into(),
            user: user.map(String::from),
            session_id: Some(session_id.into()),
            details: serde_json::json!({ "exit_code": exit_code }),
        })
        .await;
    }

    /// Log a file system operation.
    pub async fn log_fs_operation(
        &self,
        operation: &str,
        path: &str,
        user: Option<&str>,
    ) {
        self.log(&AuditEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event: format!("fs_{operation}"),
            user: user.map(String::from),
            session_id: None,
            details: serde_json::json!({ "path": path }),
        })
        .await;
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}
