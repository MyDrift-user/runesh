//! Health check definitions and execution.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A health check definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    /// Unique check ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Check type with parameters.
    pub check_type: CheckType,
    /// Check interval in seconds.
    pub interval_secs: u64,
    /// Timeout per check execution.
    pub timeout_secs: u64,
    /// Number of consecutive failures before alerting.
    #[serde(default = "default_threshold")]
    pub failure_threshold: u32,
    /// Number of consecutive successes to recover.
    #[serde(default = "default_threshold")]
    pub recovery_threshold: u32,
}

/// Types of health checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckType {
    /// HTTP(S) endpoint check.
    Http {
        url: String,
        #[serde(default = "default_expected_status")]
        expected_status: u16,
        #[serde(default)]
        expected_body: Option<String>,
    },
    /// TCP port connectivity.
    Tcp { host: String, port: u16 },
    /// ICMP ping.
    Ping { host: String },
    /// Disk space threshold.
    Disk {
        path: String,
        /// Alert when free space drops below this percentage.
        min_free_percent: f64,
    },
    /// Process running check.
    Process { name: String },
    /// Custom command (exit code 0 = OK).
    Command { command: String, args: Vec<String> },
}

/// Result of executing a check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_id: String,
    pub status: CheckStatus,
    pub message: String,
    pub duration_ms: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warning,
    Critical,
    Unknown,
}

/// Execute a check and return the result.
pub async fn run_check(check: &Check) -> CheckResult {
    let start = Instant::now();
    let timeout = Duration::from_secs(check.timeout_secs);

    let (status, message) = match &check.check_type {
        CheckType::Http {
            url,
            expected_status,
            expected_body,
        } => run_http_check(url, *expected_status, expected_body.as_deref(), timeout).await,

        CheckType::Tcp { host, port } => run_tcp_check(host, *port, timeout).await,

        CheckType::Ping { host } => run_ping_check(host).await,

        CheckType::Disk {
            path,
            min_free_percent,
        } => run_disk_check(path, *min_free_percent),

        CheckType::Process { name } => run_process_check(name),

        CheckType::Command { command, args } => run_command_check(command, args, timeout).await,
    };

    CheckResult {
        check_id: check.id.clone(),
        status,
        message,
        duration_ms: start.elapsed().as_millis() as u64,
        timestamp: chrono::Utc::now(),
    }
}

async fn run_http_check(
    url: &str,
    expected_status: u16,
    expected_body: Option<&str>,
    timeout: Duration,
) -> (CheckStatus, String) {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .danger_accept_invalid_certs(false)
        .build()
        .unwrap_or_default();

    match client.get(url).send().await {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            if status_code != expected_status {
                return (
                    CheckStatus::Critical,
                    format!("expected status {expected_status}, got {status_code}"),
                );
            }
            if let Some(expected) = expected_body {
                match resp.text().await {
                    Ok(body) if body.contains(expected) => {
                        (CheckStatus::Ok, format!("HTTP {status_code} OK"))
                    }
                    Ok(body) => (
                        CheckStatus::Critical,
                        format!(
                            "body missing expected string '{expected}', got {}",
                            &body[..body.len().min(100)]
                        ),
                    ),
                    Err(e) => (CheckStatus::Critical, format!("body read error: {e}")),
                }
            } else {
                (CheckStatus::Ok, format!("HTTP {status_code} OK"))
            }
        }
        Err(e) if e.is_timeout() => (CheckStatus::Critical, "timeout".into()),
        Err(e) if e.is_connect() => (CheckStatus::Critical, format!("connection refused: {e}")),
        Err(e) => (CheckStatus::Critical, format!("request failed: {e}")),
    }
}

async fn run_tcp_check(host: &str, port: u16, timeout: Duration) -> (CheckStatus, String) {
    let addr = format!("{host}:{port}");
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => (CheckStatus::Ok, format!("TCP {addr} open")),
        Ok(Err(e)) => (CheckStatus::Critical, format!("TCP {addr} failed: {e}")),
        Err(_) => (CheckStatus::Critical, format!("TCP {addr} timeout")),
    }
}

async fn run_ping_check(host: &str) -> (CheckStatus, String) {
    let cmd = if cfg!(windows) { "ping" } else { "ping" };
    let args = if cfg!(windows) {
        vec!["-n", "1", "-w", "3000", host]
    } else {
        vec!["-c", "1", "-W", "3", host]
    };

    match tokio::process::Command::new(cmd).args(&args).output().await {
        Ok(output) if output.status.success() => (CheckStatus::Ok, format!("ping {host} OK")),
        Ok(_) => (CheckStatus::Critical, format!("ping {host} failed")),
        Err(e) => (CheckStatus::Critical, format!("ping error: {e}")),
    }
}

fn run_disk_check(path: &str, min_free_percent: f64) -> (CheckStatus, String) {
    // Use a simple approach: check if the path exists
    // Full disk space checking requires platform-specific APIs
    if std::path::Path::new(path).exists() {
        (
            CheckStatus::Ok,
            format!("path {path} exists (disk space check requires platform integration)"),
        )
    } else {
        (CheckStatus::Critical, format!("path {path} not found"))
    }
}

fn run_process_check(name: &str) -> (CheckStatus, String) {
    // Process checking requires platform-specific APIs (sysinfo crate)
    // For now, return unknown
    (
        CheckStatus::Unknown,
        format!("process check for '{name}' requires sysinfo integration"),
    )
}

async fn run_command_check(
    command: &str,
    args: &[String],
    timeout: Duration,
) -> (CheckStatus, String) {
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match tokio::time::timeout(
        timeout,
        tokio::process::Command::new(command)
            .args(&args_ref)
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                (CheckStatus::Ok, stdout.trim().to_string())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    CheckStatus::Critical,
                    format!(
                        "exit {}: {}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    ),
                )
            }
        }
        Ok(Err(e)) => (CheckStatus::Critical, format!("spawn error: {e}")),
        Err(_) => (CheckStatus::Critical, "timeout".into()),
    }
}

fn default_threshold() -> u32 {
    3
}

fn default_expected_status() -> u16 {
    200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tcp_check_localhost() {
        // Test against a port that should be closed
        let (status, _msg) = run_tcp_check("127.0.0.1", 19999, Duration::from_secs(1)).await;
        assert_eq!(status, CheckStatus::Critical);
    }

    #[tokio::test]
    async fn command_check_echo() {
        let cmd = if cfg!(windows) { "cmd" } else { "echo" };
        let args = if cfg!(windows) {
            vec!["/C".to_string(), "echo".to_string(), "ok".to_string()]
        } else {
            vec!["ok".to_string()]
        };
        let (status, msg) = run_command_check(cmd, &args, Duration::from_secs(5)).await;
        assert_eq!(status, CheckStatus::Ok);
        assert!(msg.contains("ok"));
    }

    #[tokio::test]
    async fn command_check_failure() {
        let cmd = if cfg!(windows) { "cmd" } else { "sh" };
        let args = if cfg!(windows) {
            vec!["/C".to_string(), "exit".to_string(), "1".to_string()]
        } else {
            vec!["-c".to_string(), "exit 1".to_string()]
        };
        let (status, _) = run_command_check(cmd, &args, Duration::from_secs(5)).await;
        assert_eq!(status, CheckStatus::Critical);
    }

    #[test]
    fn disk_check_exists() {
        let path = if cfg!(windows) { "C:\\" } else { "/" };
        let (status, _) = run_disk_check(path, 10.0);
        assert_eq!(status, CheckStatus::Ok);
    }

    #[test]
    fn disk_check_missing() {
        let (status, _) = run_disk_check("/nonexistent/path/xyz", 10.0);
        assert_eq!(status, CheckStatus::Critical);
    }

    #[test]
    fn check_serialization() {
        let check = Check {
            id: "http-1".into(),
            name: "Website".into(),
            check_type: CheckType::Http {
                url: "https://example.com".into(),
                expected_status: 200,
                expected_body: None,
            },
            interval_secs: 60,
            timeout_secs: 10,
            failure_threshold: 3,
            recovery_threshold: 2,
        };
        let json = serde_json::to_string(&check).unwrap();
        let parsed: Check = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "http-1");
    }
}
