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
        _ => TaskResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("task type {:?} not implemented in executor", task.task_type),
            data: None,
            duration_ms: 0,
        },
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
