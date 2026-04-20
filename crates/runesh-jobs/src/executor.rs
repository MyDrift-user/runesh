//! Job execution engine.
//!
//! Spawns processes, captures output, enforces timeouts,
//! and produces structured results.

use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::task::{AgentTask, TaskResult, TaskStatus, TaskType};

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

/// Execute a task based on its type and parameters.
pub async fn execute_task(task: &AgentTask) -> TaskResult {
    let timeout = Duration::from_secs(task.timeout_secs);

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
            execute_command(cmd, &args, timeout, dir).await
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
            execute_command(cmd, &args, timeout, None).await
        }
        TaskType::InstallPackage => {
            let package = task
                .params
                .get("package")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (cmd, args) = platform_pkg_install(package);
            execute_command(
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
            execute_command(
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
            execute_command(
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
            execute_command(
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
            execute_command(
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
            execute_command(
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
    } else {
        // Linux: detect package manager
        if std::path::Path::new("/usr/bin/apt-get").exists() {
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
    } else {
        if std::path::Path::new("/usr/bin/apt-get").exists() {
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
}

fn platform_svc_action(service: &str, action: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        let sc_action = match action {
            "start" => "start",
            "stop" => "stop",
            "restart" => "start", // Windows: stop then start handled by caller
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
    async fn execute_task_run_command() {
        let task = AgentTask {
            id: "t1".into(),
            idempotency_key: None,
            task_type: TaskType::RunCommand,
            params: serde_json::json!({
                "command": if cfg!(windows) { "cmd" } else { "echo" },
                "args": if cfg!(windows) { vec!["/C", "echo", "task output"] } else { vec!["task output"] },
            }),
            status: TaskStatus::Running,
            retry: Default::default(),
            timeout_secs: 10,
            result: None,
        };

        let result = execute_task(&task).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("task output"));
    }

    #[tokio::test]
    async fn execute_task_run_script() {
        let script = if cfg!(windows) {
            "Write-Output 'script ran'"
        } else {
            "echo 'script ran'"
        };

        let task = AgentTask {
            id: "t2".into(),
            idempotency_key: None,
            task_type: TaskType::RunScript,
            params: serde_json::json!({ "script": script }),
            status: TaskStatus::Running,
            retry: Default::default(),
            timeout_secs: 10,
            result: None,
        };

        let result = execute_task(&task).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("script ran"));
    }
}
