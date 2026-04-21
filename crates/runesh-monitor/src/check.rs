//! Health check definitions and execution.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Policy controlling which binaries may run via `Command` checks.
///
/// An empty allowlist rejects everything. Paths must be absolute and
/// resolve to a regular file.
#[derive(Debug, Clone, Default)]
pub struct CommandCheckPolicy {
    /// Absolute paths of binaries that can be invoked.
    pub allowed_binaries: HashSet<PathBuf>,
    /// Maximum stdout+stderr bytes captured per command.
    pub max_output_bytes: usize,
    /// Inherit the parent environment (default false).
    pub inherit_env: bool,
    /// Environment keys to pass through when inherit_env is false.
    pub allowed_env_keys: HashSet<String>,
}

impl CommandCheckPolicy {
    /// Default policy: empty allowlist, 1 MiB output cap, no env inheritance.
    pub fn strict() -> Self {
        Self {
            allowed_binaries: HashSet::new(),
            max_output_bytes: 1024 * 1024,
            inherit_env: false,
            allowed_env_keys: HashSet::new(),
        }
    }
}

/// Policy controlling HTTP checks.
#[derive(Debug, Clone)]
pub struct HttpCheckPolicy {
    /// Maximum body bytes read per response.
    pub response_size_cap_bytes: usize,
}

impl Default for HttpCheckPolicy {
    fn default() -> Self {
        Self {
            response_size_cap_bytes: 10 * 1024 * 1024,
        }
    }
}

/// Shared runtime configuration injected into check execution.
#[derive(Clone)]
pub struct CheckRuntime {
    pub http_client: reqwest::Client,
    pub http_policy: HttpCheckPolicy,
    pub command_policy: Arc<CommandCheckPolicy>,
}

impl CheckRuntime {
    /// Build a runtime with a shared HTTP client and strict command policy.
    pub fn new(default_timeout: Duration) -> Self {
        Self::with_policy(default_timeout, CommandCheckPolicy::strict())
    }

    pub fn with_policy(default_timeout: Duration, command_policy: CommandCheckPolicy) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(default_timeout)
            .connect_timeout(Duration::from_secs(5))
            .danger_accept_invalid_certs(false)
            .redirect(reqwest::redirect::Policy::limited(5))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            http_client,
            http_policy: HttpCheckPolicy::default(),
            command_policy: Arc::new(command_policy),
        }
    }
}

impl Default for CheckRuntime {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

/// Execute a check and return the result, using default runtime policy.
pub async fn run_check(check: &Check) -> CheckResult {
    let runtime = CheckRuntime::new(Duration::from_secs(check.timeout_secs.max(1)));
    run_check_with(check, &runtime).await
}

/// Execute a check with an injected runtime (shared HTTP client, policy).
pub async fn run_check_with(check: &Check, runtime: &CheckRuntime) -> CheckResult {
    let start = Instant::now();
    let timeout = Duration::from_secs(check.timeout_secs);

    let (status, message) = match &check.check_type {
        CheckType::Http {
            url,
            expected_status,
            expected_body,
        } => {
            run_http_check(
                &runtime.http_client,
                &runtime.http_policy,
                url,
                *expected_status,
                expected_body.as_deref(),
                timeout,
            )
            .await
        }

        CheckType::Tcp { host, port } => run_tcp_check(host, *port, timeout).await,

        CheckType::Ping { host } => run_ping_check(host, timeout).await,

        CheckType::Disk {
            path,
            min_free_percent,
        } => run_disk_check(path, *min_free_percent),

        CheckType::Process { name } => run_process_check(name),

        CheckType::Command { command, args } => {
            run_command_check(command, args, timeout, &runtime.command_policy).await
        }
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
    client: &reqwest::Client,
    policy: &HttpCheckPolicy,
    url: &str,
    expected_status: u16,
    expected_body: Option<&str>,
    timeout: Duration,
) -> (CheckStatus, String) {
    use futures::StreamExt;

    let fut = async {
        match client.get(url).timeout(timeout).send().await {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                if status_code != expected_status {
                    return (
                        CheckStatus::Critical,
                        format!("expected status {expected_status}, got {status_code}"),
                    );
                }

                // Stream the body with a size cap to avoid unbounded memory use.
                let mut stream = resp.bytes_stream();
                let mut buf: Vec<u8> = Vec::new();
                let cap = policy.response_size_cap_bytes;
                while let Some(chunk) = stream.next().await {
                    let chunk = match chunk {
                        Ok(c) => c,
                        Err(e) => return (CheckStatus::Critical, format!("body read error: {e}")),
                    };
                    if buf.len().saturating_add(chunk.len()) > cap {
                        let take = cap.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        break;
                    } else {
                        buf.extend_from_slice(&chunk);
                    }
                }

                if let Some(expected) = expected_body {
                    let body = String::from_utf8_lossy(&buf);
                    if body.contains(expected) {
                        (CheckStatus::Ok, format!("HTTP {status_code} OK"))
                    } else {
                        let sample_len = body.len().min(100);
                        (
                            CheckStatus::Critical,
                            format!(
                                "body missing expected string '{expected}', got {}",
                                &body[..sample_len]
                            ),
                        )
                    }
                } else {
                    (CheckStatus::Ok, format!("HTTP {status_code} OK"))
                }
            }
            Err(e) if e.is_timeout() => (CheckStatus::Critical, "timeout".into()),
            Err(e) if e.is_connect() => (CheckStatus::Critical, format!("connection refused: {e}")),
            Err(e) => (CheckStatus::Critical, format!("request failed: {e}")),
        }
    };

    match tokio::time::timeout(timeout + Duration::from_secs(1), fut).await {
        Ok(r) => r,
        Err(_) => (CheckStatus::Critical, "timeout".into()),
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

/// ICMP ping implementation.
///
/// Raw ICMP requires elevated privileges on Linux/macOS, so we return
/// `Unknown` with a descriptive message when the platform path is not
/// available. Windows uses IcmpSendEcho via the IP Helper API.
async fn run_ping_check(host: &str, timeout: Duration) -> (CheckStatus, String) {
    #[cfg(windows)]
    {
        return ping_windows(host, timeout).await;
    }
    #[cfg(not(windows))]
    {
        let _ = timeout;
        return (
            CheckStatus::Unknown,
            format!(
                "ping for '{host}' requires raw ICMP privilege; grant CAP_NET_RAW or use DGRAM ICMP"
            ),
        );
    }
}

#[cfg(windows)]
async fn ping_windows(host: &str, timeout: Duration) -> (CheckStatus, String) {
    let host_owned = host.to_string();
    let tmo = timeout.as_millis().min(u32::MAX as u128) as u32;
    let join = tokio::task::spawn_blocking(move || ping_windows_blocking(&host_owned, tmo)).await;
    match join {
        Ok(Ok(())) => (CheckStatus::Ok, format!("ping {host} OK")),
        Ok(Err(msg)) => (CheckStatus::Critical, format!("ping {host} failed: {msg}")),
        Err(e) => (CheckStatus::Unknown, format!("ping task error: {e}")),
    }
}

#[cfg(windows)]
fn ping_windows_blocking(host: &str, timeout_ms: u32) -> Result<(), String> {
    use std::net::{IpAddr, ToSocketAddrs};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
    };

    let addr = (host, 0u16)
        .to_socket_addrs()
        .map_err(|e| format!("dns: {e}"))?
        .find_map(|sa| match sa.ip() {
            IpAddr::V4(v4) => Some(v4),
            _ => None,
        })
        .ok_or_else(|| "no IPv4 address".to_string())?;

    #[allow(unsafe_code)]
    unsafe {
        let handle = IcmpCreateFile();
        if handle.is_null() {
            return Err("IcmpCreateFile failed".into());
        }
        let payload = [0u8; 32];
        let mut reply = vec![0u8; 256];
        let dest: u32 = u32::from_ne_bytes(addr.octets());
        let res = IcmpSendEcho(
            handle,
            dest,
            payload.as_ptr() as *const _,
            payload.len() as u16,
            std::ptr::null_mut(),
            reply.as_mut_ptr() as *mut _,
            reply.len() as u32,
            timeout_ms,
        );
        IcmpCloseHandle(handle);
        if res == 0 {
            Err("no reply".into())
        } else {
            Ok(())
        }
    }
}

fn run_disk_check(path: &str, min_free_percent: f64) -> (CheckStatus, String) {
    use sysinfo::Disks;

    if !std::path::Path::new(path).exists() {
        return (CheckStatus::Critical, format!("path {path} not found"));
    }

    let disks = Disks::new_with_refreshed_list();

    let target = std::path::Path::new(path);
    let mut best_match: Option<(&sysinfo::Disk, usize)> = None;

    for disk in disks.list() {
        let mount = disk.mount_point();
        if target.starts_with(mount) {
            let depth = mount.components().count();
            if best_match.is_none() || depth > best_match.unwrap().1 {
                best_match = Some((disk, depth));
            }
        }
    }

    match best_match {
        Some((disk, _)) => {
            let total = disk.total_space();
            let available = disk.available_space();
            if total == 0 {
                return (
                    CheckStatus::Unknown,
                    format!("disk at {path}: 0 total bytes"),
                );
            }
            let free_percent = (available as f64 / total as f64) * 100.0;
            let total_gb = total as f64 / 1_073_741_824.0;
            let avail_gb = available as f64 / 1_073_741_824.0;

            if free_percent < min_free_percent {
                (
                    CheckStatus::Critical,
                    format!(
                        "{path}: {free_percent:.1}% free ({avail_gb:.1}/{total_gb:.1} GB), threshold {min_free_percent}%"
                    ),
                )
            } else {
                (
                    CheckStatus::Ok,
                    format!("{path}: {free_percent:.1}% free ({avail_gb:.1}/{total_gb:.1} GB)"),
                )
            }
        }
        None => (
            CheckStatus::Unknown,
            format!("no disk found for path {path}"),
        ),
    }
}

fn run_process_check(name: &str) -> (CheckStatus, String) {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let matching: Vec<_> = sys
        .processes()
        .values()
        .filter(|p| {
            let pname = p.name().to_string_lossy();
            pname.eq_ignore_ascii_case(name) || pname.to_lowercase().contains(&name.to_lowercase())
        })
        .collect();

    if matching.is_empty() {
        (CheckStatus::Critical, format!("process '{name}' not found"))
    } else {
        let pids: Vec<String> = matching
            .iter()
            .take(5)
            .map(|p| p.pid().to_string())
            .collect();
        (
            CheckStatus::Ok,
            format!(
                "process '{name}' running ({} instance(s), PIDs: {})",
                matching.len(),
                pids.join(", ")
            ),
        )
    }
}

fn resolve_binary(command: &str) -> Option<PathBuf> {
    let candidate = Path::new(command);
    if !candidate.is_absolute() {
        return None;
    }
    std::fs::canonicalize(candidate).ok()
}

async fn run_command_check(
    command: &str,
    args: &[String],
    timeout: Duration,
    policy: &CommandCheckPolicy,
) -> (CheckStatus, String) {
    if policy.allowed_binaries.is_empty() {
        return (
            CheckStatus::Critical,
            "command checks disabled: empty binary allowlist".into(),
        );
    }

    let resolved = match resolve_binary(command) {
        Some(p) => p,
        None => {
            return (
                CheckStatus::Critical,
                format!("command '{command}' must be an absolute path"),
            );
        }
    };

    if !resolved.is_file() {
        return (
            CheckStatus::Critical,
            format!("command '{}' is not a regular file", resolved.display()),
        );
    }

    let allowlist: HashSet<PathBuf> = policy
        .allowed_binaries
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();

    if !allowlist.contains(&resolved) {
        return (
            CheckStatus::Critical,
            format!("binary '{}' not in allowlist", resolved.display()),
        );
    }

    let mut cmd = tokio::process::Command::new(&resolved);
    cmd.args(args);
    if !policy.inherit_env {
        cmd.env_clear();
    }
    for key in &policy.allowed_env_keys {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    #[cfg(unix)]
    {
        #[allow(unsafe_code)]
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (CheckStatus::Critical, format!("spawn error: {e}")),
    };

    let pid = child.id();

    #[cfg(windows)]
    let _job_guard = pid.and_then(|p| windows_job::assign_to_new_job(p).ok());

    // Take stdout/stderr handles so we can await them alongside wait().
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let wait_fut = async move {
        use tokio::io::AsyncReadExt;
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut out).await;
        }
        if let Some(mut s) = stderr {
            let _ = s.read_to_end(&mut err).await;
        }
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, out, err))
    };

    let result = tokio::time::timeout(timeout, wait_fut).await;

    match result {
        Ok(Ok((status, out, err))) => {
            let stdout_b = truncate_bytes(&out, policy.max_output_bytes);
            let stderr_b = truncate_bytes(&err, policy.max_output_bytes);
            if status.success() {
                (
                    CheckStatus::Ok,
                    String::from_utf8_lossy(&stdout_b).trim().to_string(),
                )
            } else {
                (
                    CheckStatus::Critical,
                    format!(
                        "exit {}: {}",
                        status.code().unwrap_or(-1),
                        String::from_utf8_lossy(&stderr_b).trim()
                    ),
                )
            }
        }
        Ok(Err(e)) => (CheckStatus::Critical, format!("io error: {e}")),
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                #[allow(unsafe_code)]
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                }
            }
            #[cfg(windows)]
            {
                let _ = pid;
            }
            (CheckStatus::Critical, "timeout".into())
        }
    }
}

fn truncate_bytes(b: &[u8], cap: usize) -> Vec<u8> {
    if b.len() <= cap {
        b.to_vec()
    } else {
        b[..cap].to_vec()
    }
}

#[cfg(windows)]
mod windows_job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};

    pub struct JobGuard(HANDLE);

    impl Drop for JobGuard {
        fn drop(&mut self) {
            #[allow(unsafe_code)]
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn assign_to_new_job(pid: u32) -> Result<JobGuard, ()> {
        #[allow(unsafe_code)]
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                CloseHandle(job);
                return Err(());
            }
            let proc = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
            if proc.is_null() {
                CloseHandle(job);
                return Err(());
            }
            let ok = AssignProcessToJobObject(job, proc) != 0;
            CloseHandle(proc);
            if !ok {
                CloseHandle(job);
                return Err(());
            }
            Ok(JobGuard(job))
        }
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

    fn strict_policy() -> CommandCheckPolicy {
        CommandCheckPolicy::strict()
    }

    #[tokio::test]
    async fn tcp_check_localhost() {
        let (status, _msg) = run_tcp_check("127.0.0.1", 19999, Duration::from_secs(1)).await;
        assert_eq!(status, CheckStatus::Critical);
    }

    #[tokio::test]
    async fn command_rejects_empty_allowlist() {
        let policy = strict_policy();
        let (status, msg) =
            run_command_check("/bin/echo", &["ok".into()], Duration::from_secs(2), &policy).await;
        assert_eq!(status, CheckStatus::Critical);
        assert!(msg.contains("allowlist"), "msg={msg}");
    }

    #[tokio::test]
    async fn command_rejects_unlisted_binary() {
        let allowed_probe = if cfg!(windows) {
            std::path::PathBuf::from(r"C:\Windows\System32\cmd.exe")
        } else {
            std::path::PathBuf::from("/bin/true")
        };
        let mut policy = strict_policy();
        policy.allowed_binaries.insert(allowed_probe);

        let other = if cfg!(windows) {
            r"C:\Windows\System32\where.exe"
        } else {
            "/bin/echo"
        };
        let (status, msg) = run_command_check(other, &[], Duration::from_secs(2), &policy).await;
        assert_eq!(status, CheckStatus::Critical);
        assert!(
            msg.contains("allowlist") || msg.contains("not a regular file"),
            "msg={msg}"
        );
    }

    #[tokio::test]
    async fn command_allows_listed_binary() {
        let (bin, args) = if cfg!(windows) {
            (
                std::path::PathBuf::from(r"C:\Windows\System32\cmd.exe"),
                vec!["/C".to_string(), "echo ok".to_string()],
            )
        } else {
            (
                std::path::PathBuf::from("/bin/echo"),
                vec!["ok".to_string()],
            )
        };
        if !bin.exists() {
            eprintln!("skipping: {} missing", bin.display());
            return;
        }
        let mut policy = strict_policy();
        policy.allowed_binaries.insert(bin.clone());

        let (status, msg) = run_command_check(
            bin.to_str().unwrap(),
            &args,
            Duration::from_secs(5),
            &policy,
        )
        .await;
        assert_eq!(status, CheckStatus::Ok, "msg={msg}");
        assert!(msg.contains("ok"), "msg={msg}");
    }

    #[tokio::test]
    async fn command_relative_path_rejected() {
        let mut policy = strict_policy();
        policy
            .allowed_binaries
            .insert(std::path::PathBuf::from("echo"));
        let (status, msg) =
            run_command_check("echo", &["hi".into()], Duration::from_secs(2), &policy).await;
        assert_eq!(status, CheckStatus::Critical);
        assert!(msg.contains("absolute"), "msg={msg}");
    }

    #[tokio::test]
    async fn http_check_caps_response_body() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5000000\r\n\r\n",
                    )
                    .await;
                let chunk = vec![0u8; 4096];
                for _ in 0..2000 {
                    if sock.write_all(&chunk).await.is_err() {
                        break;
                    }
                }
            }
        });

        let url = format!("http://{}/", addr);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let policy = HttpCheckPolicy {
            response_size_cap_bytes: 16 * 1024,
        };
        let (status, _msg) =
            run_http_check(&client, &policy, &url, 200, None, Duration::from_secs(5)).await;
        assert_eq!(status, CheckStatus::Ok);
    }

    #[test]
    fn disk_check_exists() {
        let path = if cfg!(windows) { "C:\\" } else { "/" };
        let (status, msg) = run_disk_check(path, 0.1);
        assert_eq!(status, CheckStatus::Ok, "disk check failed: {msg}");
        assert!(msg.contains("free"), "msg={msg}");
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
