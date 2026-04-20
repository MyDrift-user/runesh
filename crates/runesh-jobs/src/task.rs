//! Agent task model.
//!
//! The controller pushes tasks to agents. Each task has a type,
//! parameters, and lifecycle tracking.

use serde::{Deserialize, Serialize};

/// A task pushed from the controller to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Unique task ID.
    pub id: String,

    /// Idempotency key (prevents duplicate execution).
    #[serde(default)]
    pub idempotency_key: Option<String>,

    /// Task type.
    pub task_type: TaskType,

    /// Task parameters (type-specific JSON).
    #[serde(default)]
    pub params: serde_json::Value,

    /// Current status.
    pub status: TaskStatus,

    /// Retry policy.
    #[serde(default)]
    pub retry: RetryPolicy,

    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Result (populated after execution).
    #[serde(default)]
    pub result: Option<TaskResult>,
}

/// Task types that an agent can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Collect and report inventory.
    CollectInventory,
    /// Run a shell command.
    RunCommand,
    /// Run a script (PowerShell/Bash/Python).
    RunScript,
    /// Install a package.
    InstallPackage,
    /// Remove a package.
    RemovePackage,
    /// Start a service.
    StartService,
    /// Stop a service.
    StopService,
    /// Restart a service.
    RestartService,
    /// Apply a configuration baseline.
    ApplyBaseline,
    /// Check compliance against a baseline.
    CheckCompliance,
    /// Update the agent binary.
    SelfUpdate,
    /// Rotate the node key.
    RotateNodeKey,
    /// Collect a diagnostic bundle.
    DiagnosticBundle,
    /// Schedule a reboot.
    ScheduleReboot,
    /// Custom task type.
    Custom(String),
}

/// Task lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Queued, waiting to execute.
    Queued,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed (see result for details).
    Failed,
    /// Cancelled before execution.
    Cancelled,
    /// Skipped (idempotency key already seen).
    Skipped,
}

/// Retry policy for failed tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Current retry count.
    pub retry_count: u32,
    /// Delay between retries in seconds.
    pub delay_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_count: 0,
            delay_secs: 30,
        }
    }
}

/// Result of a task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Exit code (0 = success).
    pub exit_code: i32,
    /// Standard output.
    #[serde(default)]
    pub stdout: String,
    /// Standard error.
    #[serde(default)]
    pub stderr: String,
    /// Structured output (task-type-specific).
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

impl AgentTask {
    /// Whether this task can be retried.
    pub fn can_retry(&self) -> bool {
        self.status == TaskStatus::Failed && self.retry.retry_count < self.retry.max_retries
    }

    /// Mark as running.
    pub fn start(&mut self) {
        self.status = TaskStatus::Running;
    }

    /// Mark as completed with a result.
    pub fn complete(&mut self, result: TaskResult) {
        self.status = if result.exit_code == 0 {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        self.result = Some(result);
    }

    /// Increment retry count.
    pub fn record_retry(&mut self) {
        self.retry.retry_count += 1;
    }
}

/// A task queue that tracks pending and completed tasks.
#[derive(Debug, Default)]
pub struct TaskQueue {
    tasks: Vec<AgentTask>,
    /// Set of idempotency keys already seen.
    seen_keys: std::collections::HashSet<String>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a task. Returns false if the idempotency key was already seen.
    pub fn enqueue(&mut self, mut task: AgentTask) -> bool {
        if let Some(key) = &task.idempotency_key {
            if !self.seen_keys.insert(key.clone()) {
                task.status = TaskStatus::Skipped;
                self.tasks.push(task);
                return false;
            }
        }
        task.status = TaskStatus::Queued;
        self.tasks.push(task);
        true
    }

    /// Get the next queued task.
    pub fn next_pending(&mut self) -> Option<&mut AgentTask> {
        self.tasks
            .iter_mut()
            .find(|t| t.status == TaskStatus::Queued)
    }

    /// Number of pending tasks.
    pub fn pending_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Queued)
            .count()
    }

    /// Number of completed tasks.
    pub fn completed_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count()
    }

    /// Drain completed and failed tasks older than the retention limit.
    pub fn prune(&mut self, max_completed: usize) {
        let mut completed = 0;
        self.tasks.retain(|t| {
            if t.status == TaskStatus::Completed || t.status == TaskStatus::Failed {
                completed += 1;
                completed <= max_completed
            } else {
                true
            }
        });
    }
}

fn default_timeout() -> u64 {
    300
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(id: &str) -> AgentTask {
        AgentTask {
            id: id.into(),
            idempotency_key: None,
            task_type: TaskType::CollectInventory,
            params: serde_json::Value::Null,
            status: TaskStatus::Queued,
            retry: RetryPolicy::default(),
            timeout_secs: 60,
            result: None,
        }
    }

    #[test]
    fn task_lifecycle() {
        let mut task = sample_task("t1");
        assert_eq!(task.status, TaskStatus::Queued);

        task.start();
        assert_eq!(task.status, TaskStatus::Running);

        task.complete(TaskResult {
            exit_code: 0,
            stdout: "ok".into(),
            stderr: String::new(),
            data: None,
            duration_ms: 150,
        });
        assert_eq!(task.status, TaskStatus::Completed);
    }

    #[test]
    fn task_failure_and_retry() {
        let mut task = sample_task("t1");
        task.start();
        task.complete(TaskResult {
            exit_code: 1,
            stdout: String::new(),
            stderr: "error".into(),
            data: None,
            duration_ms: 50,
        });
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.can_retry());

        task.record_retry();
        task.record_retry();
        task.record_retry();
        assert!(!task.can_retry());
    }

    #[test]
    fn queue_enqueue_and_dequeue() {
        let mut queue = TaskQueue::new();
        queue.enqueue(sample_task("t1"));
        queue.enqueue(sample_task("t2"));

        assert_eq!(queue.pending_count(), 2);

        let task = queue.next_pending().unwrap();
        assert_eq!(task.id, "t1");
        task.start();

        assert_eq!(queue.pending_count(), 1);
    }

    #[test]
    fn idempotency_dedup() {
        let mut queue = TaskQueue::new();

        let mut t1 = sample_task("t1");
        t1.idempotency_key = Some("key-abc".into());
        assert!(queue.enqueue(t1));

        let mut t2 = sample_task("t2");
        t2.idempotency_key = Some("key-abc".into());
        assert!(!queue.enqueue(t2)); // duplicate key

        assert_eq!(queue.pending_count(), 1);
    }

    #[test]
    fn all_task_types_serialize() {
        let types = vec![
            TaskType::CollectInventory,
            TaskType::RunCommand,
            TaskType::RunScript,
            TaskType::InstallPackage,
            TaskType::SelfUpdate,
            TaskType::ScheduleReboot,
            TaskType::Custom("my_task".into()),
        ];
        for tt in types {
            let json = serde_json::to_string(&tt).unwrap();
            let parsed: TaskType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, tt);
        }
    }

    #[test]
    fn prune_completed() {
        let mut queue = TaskQueue::new();
        for i in 0..10 {
            let t = sample_task(&format!("t{i}"));
            queue.enqueue(t);
        }
        // Complete all
        while let Some(task) = queue.next_pending() {
            task.start();
            task.complete(TaskResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                data: None,
                duration_ms: 10,
            });
        }
        assert_eq!(queue.completed_count(), 10);

        queue.prune(5);
        assert_eq!(queue.completed_count(), 5);
    }
}
