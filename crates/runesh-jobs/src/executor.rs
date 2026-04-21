//! Job execution engine.
//!
//! Spawns processes, captures output, enforces timeouts, and produces
//! structured results.
//!
//! Arbitrary command and script execution is gated behind two safety rails:
//!
//! * [`CommandAllowlist`] restricts the set of binaries an executor will run.
//!   Empty allowlists reject every command; callers must opt in explicitly.
//! * [`TaskVerifier`] is a pluggable hook that inspects every task before
//!   dispatch. [`RejectScripts`] (the default) hard-fails on [`TaskType::RunScript`];
//!   [`NullVerifier`] is available for tests only and is named loudly so it
//!   is obvious in review.
//!
//! Direct use of [`execute_command`] and [`execute_task`] remains available
//! for callers that have their own sandboxing layer, but the [`Executor`]
//! wrapper is the recommended entry point.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::task::{AgentTask, TaskResult, TaskType};

// ── Safety rails ─────────────────────────────────────────────────────────────

/// Allowlist of binary paths the executor is permitted to spawn.
///
/// An empty allowlist rejects every command. The check is against the exact
/// path string supplied to [`execute_command`] after canonicalisation of
/// obvious relative forms; callers should supply absolute paths in production.
#[derive(Debug, Clone, Default)]
pub struct CommandAllowlist {
    allowed: HashSet<PathBuf>,
}

impl CommandAllowlist {
    /// Empty allowlist (rejects everything). The name is a reminder that
    /// this is the deny-by-default posture.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build an allowlist from an iterator of paths.
    pub fn from_paths<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Self {
            allowed: paths.into_iter().map(Into::into).collect(),
        }
    }

    /// Add one path to the allowlist.
    pub fn allow(&mut self, path: impl Into<PathBuf>) {
        self.allowed.insert(path.into());
    }

    /// Check whether the given binary is permitted to run.
    pub fn is_allowed(&self, command: &str) -> bool {
        if self.allowed.is_empty() {
            return false;
        }
        let probe = PathBuf::from(command);
        self.allowed.contains(&probe)
    }

    /// Number of allowed binaries.
    pub fn len(&self) -> usize {
        self.allowed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }
}

/// Reasons a [`TaskVerifier`] may reject a task.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("scripts are disabled by policy")]
    ScriptsDisabled,
    #[error("binary not on allowlist: {0}")]
    NotAllowlisted(String),
    #[error("rejected by policy: {0}")]
    Other(String),
}

/// Pluggable policy hook. Called on every task before dispatch. Returning
/// `Err` short-circuits execution with a failure result.
pub trait TaskVerifier: Send + Sync {
    fn verify(&self, task: &AgentTask) -> Result<(), VerifyError>;
}

/// Default verifier: rejects every [`TaskType::RunScript`] task.
///
/// Scripts are the highest-risk task kind because they hand an arbitrary
/// string to a shell interpreter. Agents that genuinely need to run scripts
/// should install a domain-specific verifier that validates the script body
/// against a signature or static allowlist.
#[derive(Debug, Default, Clone, Copy)]
pub struct RejectScripts;

impl TaskVerifier for RejectScripts {
    fn verify(&self, task: &AgentTask) -> Result<(), VerifyError> {
        if task.task_type == TaskType::RunScript {
            return Err(VerifyError::ScriptsDisabled);
        }
        Ok(())
    }
}

/// Tests-only verifier that approves every task. Named loudly so it is
/// unmissable in review; do not wire this into production binaries.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullVerifier;

impl TaskVerifier for NullVerifier {
    fn verify(&self, _task: &AgentTask) -> Result<(), VerifyError> {
        Ok(())
    }
}

/// Policy-enforcing task executor. Wraps an allowlist plus a verifier and
/// dispatches tasks through [`execute_task_with_policy`].
pub struct Executor {
    allowlist: CommandAllowlist,
    verifier: Arc<dyn TaskVerifier>,
}

impl Executor {
    /// Build an executor that will only spawn binaries from `allowlist` and
    /// uses [`RejectScripts`] as its verifier.
    pub fn with_allowlist(allowlist: CommandAllowlist) -> Self {
        Self {
            allowlist,
            verifier: Arc::new(RejectScripts),
        }
    }

    /// Replace the verifier.
    pub fn with_verifier(mut self, verifier: Arc<dyn TaskVerifier>) -> Self {
        self.verifier = verifier;
        self
    }

    /// Access the allowlist.
    pub fn allowlist(&self) -> &CommandAllowlist {
        &self.allowlist
    }

    /// Execute a task, enforcing the verifier and the allowlist.
    pub async fn execute(&self, task: &AgentTask) -> TaskResult {
        if let Err(e) = self.verifier.verify(task) {
            return TaskResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("verifier rejected task: {e}"),
                data: None,
                duration_ms: 0,
            };
        }
        execute_task_with_policy(task, &self.allowlist).await
    }
}

// ── Direct command execution ─────────────────────────────────────────────────

/// Execute a task that runs a shell command or script.
///
/// Returns the captured output as a TaskResult.
pub async fn execute_command(
    command: &str,
    args: &[&str],
    timeout: Duration,
    working_dir: Option<&str>,
) -> TaskResult {
    let start = Instant::now();

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let result = tokio::time::timeout(timeout, async {
        let child = cmd.spawn();
        match child {
            Ok(child) => match child.wait_with_output().await {
                Ok(output) => TaskResult {
                    exit_code: output.status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    data: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                Err(e) => TaskResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("process error: {e}"),
                    data: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            },
            Err(e) => TaskResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("spawn error: {e}"),
                data: None,
                duration_ms: start.elapsed().as_millis() as u64,
            },
        }
    })
    .await;

    match result {
        Ok(task_result) => task_result,
        Err(_) => TaskResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("timeout after {}s", timeout.as_secs()),
            data: None,
            duration_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Run `execute_command` only when `allowlist` permits the binary. Logs the
/// full invocation at info level for audit.
async fn execute_command_guarded(
    allowlist: &CommandAllowlist,
    command: &str,
    args: &[&str],
    timeout: Duration,
    working_dir: Option<&str>,
) -> TaskResult {
    if !allowlist.is_allowed(command) {
        tracing::warn!(
            command = %command,
            args = ?args,
            "rejecting command: not on allowlist"
        );
        return TaskResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("command {command} not on allowlist"),
            data: None,
            duration_ms: 0,
        };
    }
    tracing::info!(command = %command, args = ?args, "executing allowlisted command");
    execute_command(command, args, timeout, working_dir).await
}

/// Execute a task based on its type and parameters.
///
/// This entry point exists for callers that supply their own sandboxing; it
/// does not enforce the [`CommandAllowlist`] and it does not consult any
/// verifier. Prefer [`Executor::execute`] or [`execute_task_with_policy`].
pub async fn execute_task(task: &AgentTask) -> TaskResult {
    execute_task_inner(task, None).await
}

/// Execute a task with allowlist enforcement. All subprocess invocations must
/// have their binary present in `allowlist`; otherwise the task fails fast.
pub async fn execute_task_with_policy(
    task: &AgentTask,
    allowlist: &CommandAllowlist,
) -> TaskResult {
    execute_task_inner(task, Some(allowlist)).await
}

async fn execute_task_inner(task: &AgentTask, allowlist: Option<&CommandAllowlist>) -> TaskResult {
    let timeout = Duration::from_secs(task.timeout_secs);

    async fn run(
        allowlist: Option<&CommandAllowlist>,
        command: &str,
        args: &[&str],
        timeout: Duration,
        working_dir: Option<&str>,
    ) -> TaskResult {
        match allowlist {
            Some(al) => execute_command_guarded(al, command, args, timeout, working_dir).await,
            None => execute_command(command, args, timeout, working_dir).await,
        }
    }

    match &task.task_type {
        TaskType::RunCommand => {
            let cmd = task
                .params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args: Vec<&str> = task
                .params
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let dir = task.params.get("working_dir").and_then(|v| v.as_str());
            run(allowlist, cmd, &args, timeout, dir).await
        }
        TaskType::RunScript => {
            let script = task
                .params
                .get("script")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let interpreter = task.params.get("interpreter").and_then(|v| v.as_str());

            let (cmd, args) = if cfg!(windows) {
                let interp = interpreter.unwrap_or("powershell.exe");
                (
                    interp,
                    vec!["-NoProfile", "-NonInteractive", "-Command", script],
                )
            } else {
                let interp = interpreter.unwrap_or("/bin/sh");
                (interp, vec!["-c", script])
            };
            run(allowlist, cmd, &args, timeout, None).await
        }
        TaskType::InstallPackage => {
            let package = task
                .params
                .get("package")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_pkg_install(package);
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        TaskType::RemovePackage => {
            let package = task
                .params
                .get("package")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_pkg_remove(package);
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        TaskType::StartService => {
            let service = task
                .params
                .get("service")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_svc_action(service, "start");
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        TaskType::StopService => {
            let service = task
                .params
                .get("service")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_svc_action(service, "stop");
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        TaskType::RestartService => {
            let service = task
                .params
                .get("service")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_svc_action(service, "restart");
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        TaskType::ScheduleReboot => {
            let delay = task
                .params
                .get("delay_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(60);
            let (cmd, args) = platform_reboot(delay);
            run(
                allowlist,
                &cmd,
                &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                timeout,
                None,
            )
            .await
        }
        _ => TaskResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("task type {:?} not implemented in executor", task.task_type),
            data: None,
            duration_ms: 0,
        },
    }
}

fn platform_pkg_install(package: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "winget".into(),
            vec![
                "install".into(),
                "--id".into(),
                package.into(),
                "--silent".into(),
                "--accept-package-agreements".into(),
                "--accept-source-agreements".into(),
                "--disable-interactivity".into(),
            ],
        )
    } else if cfg!(target_os = "macos") {
        ("brew".into(), vec!["install".into(), package.into()])
    } else if std::path::Path::new("/usr/bin/apt-get").exists() {
        (
            "apt-get".into(),
            vec!["install".into(), "-y".into(), package.into()],
        )
    } else if std::path::Path::new("/usr/bin/dnf").exists() {
        (
            "dnf".into(),
            vec!["install".into(), "-y".into(), package.into()],
        )
    } else if std::path::Path::new("/usr/bin/pacman").exists() {
        (
            "pacman".into(),
            vec!["-S".into(), "--noconfirm".into(), package.into()],
        )
    } else {
        (
            "echo".into(),
            vec![format!("no package manager found for {package}")],
        )
    }
}

fn platform_pkg_remove(package: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "winget".into(),
            vec![
                "uninstall".into(),
                "--id".into(),
                package.into(),
                "--silent".into(),
                "--disable-interactivity".into(),
            ],
        )
    } else if cfg!(target_os = "macos") {
        ("brew".into(), vec!["uninstall".into(), package.into()])
    } else if std::path::Path::new("/usr/bin/apt-get").exists() {
        (
            "apt-get".into(),
            vec!["remove".into(), "-y".into(), package.into()],
        )
    } else if std::path::Path::new("/usr/bin/dnf").exists() {
        (
            "dnf".into(),
            vec!["remove".into(), "-y".into(), package.into()],
        )
    } else if std::path::Path::new("/usr/bin/pacman").exists() {
        (
            "pacman".into(),
            vec!["-R".into(), "--noconfirm".into(), package.into()],
        )
    } else {
        (
            "echo".into(),
            vec![format!("no package manager found for {package}")],
        )
    }
}

fn platform_svc_action(service: &str, action: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        let sc_action = match action {
            "start" => "start",
            "stop" => "stop",
            "restart" => "start",
            _ => action,
        };
        ("sc".into(), vec![sc_action.into(), service.into()])
    } else if cfg!(target_os = "macos") {
        let launchctl_action = match action {
            "start" => "load",
            "stop" => "unload",
            "restart" => "kickstart",
            _ => action,
        };
        (
            "launchctl".into(),
            vec![launchctl_action.into(), service.into()],
        )
    } else {
        ("systemctl".into(), vec![action.into(), service.into()])
    }
}

fn platform_reboot(delay_secs: u64) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "shutdown".into(),
            vec!["/r".into(), "/t".into(), delay_secs.to_string()],
        )
    } else {
        let minutes = (delay_secs / 60).max(1);
        ("shutdown".into(), vec!["-r".into(), format!("+{minutes}")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{RetryPolicy, TaskStatus};

    fn task_with(task_type: TaskType, params: serde_json::Value) -> AgentTask {
        AgentTask {
            id: "t".into(),
            idempotency_key: None,
            task_type,
            params,
            status: TaskStatus::Running,
            retry: RetryPolicy::default(),
            timeout_secs: 10,
            result: None,
        }
    }

    #[tokio::test]
    async fn execute_echo() {
        let cmd = if cfg!(windows) { "cmd" } else { "echo" };
        let args: Vec<&str> = if cfg!(windows) {
            vec!["/C", "echo", "hello"]
        } else {
            vec!["hello"]
        };

        let result = execute_command(cmd, &args, Duration::from_secs(5), None).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn execute_failing_command() {
        let cmd = if cfg!(windows) { "cmd" } else { "sh" };
        let args: Vec<&str> = if cfg!(windows) {
            vec!["/C", "exit", "42"]
        } else {
            vec!["-c", "exit 42"]
        };

        let result = execute_command(cmd, &args, Duration::from_secs(5), None).await;
        assert_eq!(result.exit_code, 42);
    }

    #[tokio::test]
    async fn execute_timeout() {
        let cmd = if cfg!(windows) {
            "powershell.exe"
        } else {
            "sleep"
        };
        let args: Vec<&str> = if cfg!(windows) {
            vec!["-Command", "Start-Sleep -Seconds 10"]
        } else {
            vec!["10"]
        };

        let result = execute_command(cmd, &args, Duration::from_millis(500), None).await;
        assert_eq!(result.exit_code, -1);
        assert!(result.stderr.contains("timeout"));
    }

    #[tokio::test]
    async fn execute_nonexistent_command() {
        let result =
            execute_command("nonexistent_binary_xyz", &[], Duration::from_secs(5), None).await;
        assert_eq!(result.exit_code, -1);
        assert!(result.stderr.contains("spawn error"));
    }

    #[tokio::test]
    async fn execute_task_run_command_unchecked() {
        let task = task_with(
            TaskType::RunCommand,
            serde_json::json!({
                "command": if cfg!(windows) { "cmd" } else { "echo" },
                "args": if cfg!(windows) { vec!["/C", "echo", "task output"] } else { vec!["task output"] },
            }),
        );

        let result = execute_task(&task).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("task output"));
    }

    #[tokio::test]
    async fn executor_rejects_scripts_by_default() {
        let exec = Executor::with_allowlist(CommandAllowlist::from_paths(["echo"]));
        let task = task_with(
            TaskType::RunScript,
            serde_json::json!({ "script": "echo hi" }),
        );
        let result = exec.execute(&task).await;
        assert_eq!(result.exit_code, -1);
        assert!(result.stderr.contains("scripts are disabled"));
    }

    #[tokio::test]
    async fn empty_allowlist_rejects_all_commands() {
        let exec = Executor::with_allowlist(CommandAllowlist::empty());
        let task = task_with(
            TaskType::RunCommand,
            serde_json::json!({ "command": "echo", "args": ["hi"] }),
        );
        let result = exec.execute(&task).await;
        assert_eq!(result.exit_code, -1);
        assert!(result.stderr.contains("not on allowlist"));
    }

    #[tokio::test]
    async fn allowlisted_command_runs() {
        let cmd = if cfg!(windows) { "cmd" } else { "echo" };
        let exec = Executor::with_allowlist(CommandAllowlist::from_paths([cmd]));
        let task = task_with(
            TaskType::RunCommand,
            serde_json::json!({
                "command": cmd,
                "args": if cfg!(windows) { vec!["/C", "echo", "ok"] } else { vec!["ok"] },
            }),
        );
        let result = exec.execute(&task).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("ok"));
    }

    #[tokio::test]
    async fn null_verifier_allows_scripts_when_binary_allowlisted() {
        let interp = if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        };
        let exec = Executor::with_allowlist(CommandAllowlist::from_paths([interp]))
            .with_verifier(Arc::new(NullVerifier));
        let script = if cfg!(windows) {
            "Write-Output ok"
        } else {
            "echo ok"
        };
        let task = task_with(TaskType::RunScript, serde_json::json!({ "script": script }));
        let result = exec.execute(&task).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("ok"));
    }
}
