//! Agent task model.
//!
//! The controller pushes tasks to agents. Each task has a type,
//! parameters, and lifecycle tracking.

use std::collections::HashMap;
use std::time::{Duration, Instant};

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

/// Backoff strategy for retry delays.
///
/// Exponential strategies compute the delay as `base * factor^attempt`,
/// clamped to `max`. The jitter variant additionally multiplies the result
/// by a uniform random factor in `[0.5, 1.5)` to spread retry storms.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BackoffStrategy {
    /// Fixed delay between attempts.
    Fixed,
    /// Exponential backoff, no randomness.
    Exponential {
        #[serde(with = "duration_secs")]
        base: Duration,
        factor: f32,
        #[serde(with = "duration_secs")]
        max: Duration,
    },
    /// Exponential backoff with decorrelated jitter.
    ExponentialJitter {
        #[serde(with = "duration_secs")]
        base: Duration,
        factor: f32,
        #[serde(with = "duration_secs")]
        max: Duration,
    },
}

impl BackoffStrategy {
    /// Default: exponential with jitter, base=1s, factor=2.0, cap=300s.
    pub fn default_jittered() -> Self {
        Self::ExponentialJitter {
            base: Duration::from_secs(1),
            factor: 2.0,
            max: Duration::from_secs(300),
        }
    }

    fn exp_raw(base: Duration, factor: f32, max: Duration, attempt: u32) -> Duration {
        let attempt = attempt.min(32);
        let base_ms = base.as_millis() as f64;
        let factor = factor.max(1.0) as f64;
        let raw_ms = base_ms * factor.powi(attempt as i32);
        let cap_ms = max.as_millis() as f64;
        let clamped = raw_ms.min(cap_ms);
        // Guard against NaN/inf.
        if !clamped.is_finite() || clamped < 0.0 {
            return max;
        }
        Duration::from_millis(clamped as u64)
    }

    /// Compute the delay before the given retry attempt, using the provided
    /// fixed fallback delay when the strategy is [`BackoffStrategy::Fixed`].
    pub fn next_retry_delay(&self, attempt: u32, fixed_fallback: Duration) -> Duration {
        match *self {
            BackoffStrategy::Fixed => fixed_fallback,
            BackoffStrategy::Exponential { base, factor, max } => {
                Self::exp_raw(base, factor, max, attempt)
            }
            BackoffStrategy::ExponentialJitter { base, factor, max } => {
                let deterministic = Self::exp_raw(base, factor, max, attempt);
                let jitter: f64 = {
                    use rand::Rng;
                    rand::thread_rng().gen_range(0.5f64..1.5f64)
                };
                let ms = (deterministic.as_millis() as f64 * jitter) as u64;
                Duration::from_millis(ms).min(max)
            }
        }
    }
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        Self::default_jittered()
    }
}

mod duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

/// Retry policy for failed tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Current retry count.
    pub retry_count: u32,
    /// Fallback delay in seconds, used when [`backoff`](Self::backoff) is
    /// [`BackoffStrategy::Fixed`].
    pub delay_secs: u64,
    /// Backoff strategy. Default: exponential with jitter, base=1s, cap=300s.
    #[serde(default)]
    pub backoff: BackoffStrategy,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_count: 0,
            delay_secs: 30,
            backoff: BackoffStrategy::default_jittered(),
        }
    }
}

impl RetryPolicy {
    /// Compute the delay before the next retry attempt.
    pub fn next_delay(&self) -> Duration {
        self.backoff
            .next_retry_delay(self.retry_count, Duration::from_secs(self.delay_secs))
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
///
/// Idempotency keys are remembered for the configured [`TaskQueue::idempotency_window`]
/// and then evicted lazily on the next mutating call. This keeps memory bounded
/// for agents that run continuously without restarts.
#[derive(Debug)]
pub struct TaskQueue {
    tasks: Vec<AgentTask>,
    /// Map of idempotency key to the time it was first seen. Entries older
    /// than `idempotency_window` are dropped on the next enqueue.
    seen_keys: HashMap<String, Instant>,
    /// How long to remember idempotency keys.
    pub idempotency_window: Duration,
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            seen_keys: HashMap::new(),
            idempotency_window: Duration::from_secs(86_400),
        }
    }
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a queue with a custom idempotency window.
    pub fn with_idempotency_window(window: Duration) -> Self {
        Self {
            idempotency_window: window,
            ..Self::default()
        }
    }

    fn evict_expired_keys(&mut self) {
        let cutoff = Instant::now();
        let window = self.idempotency_window;
        self.seen_keys
            .retain(|_, seen_at| cutoff.duration_since(*seen_at) < window);
    }

    /// Enqueue a task. Returns false if the idempotency key was already seen
    /// within the current window.
    pub fn enqueue(&mut self, mut task: AgentTask) -> bool {
        self.evict_expired_keys();
        if let Some(key) = &task.idempotency_key {
            if let Some(seen_at) = self.seen_keys.get(key)
                && Instant::now().duration_since(*seen_at) < self.idempotency_window
            {
                task.status = TaskStatus::Skipped;
                self.tasks.push(task);
                return false;
            }
            self.seen_keys.insert(key.clone(), Instant::now());
        }
        task.status = TaskStatus::Queued;
        self.tasks.push(task);
        true
    }

    /// Number of remembered idempotency keys (for diagnostics).
    pub fn idempotency_key_count(&self) -> usize {
        self.seen_keys.len()
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
        assert!(!queue.enqueue(t2));

        assert_eq!(queue.pending_count(), 1);
    }

    #[test]
    fn idempotency_keys_evict_after_window() {
        // With a near-zero window, the second enqueue of the same key
        // should succeed because the first has already expired.
        let mut queue = TaskQueue::with_idempotency_window(Duration::from_millis(1));

        let mut t1 = sample_task("t1");
        t1.idempotency_key = Some("key-x".into());
        assert!(queue.enqueue(t1));

        std::thread::sleep(Duration::from_millis(5));

        let mut t2 = sample_task("t2");
        t2.idempotency_key = Some("key-x".into());
        assert!(queue.enqueue(t2));
        // First insert should have been evicted before the second was recorded.
        assert_eq!(queue.idempotency_key_count(), 1);
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

    #[test]
    fn exponential_backoff_caps() {
        let bo = BackoffStrategy::Exponential {
            base: Duration::from_secs(1),
            factor: 2.0,
            max: Duration::from_secs(30),
        };
        assert_eq!(
            bo.next_retry_delay(0, Duration::ZERO),
            Duration::from_secs(1)
        );
        assert_eq!(
            bo.next_retry_delay(1, Duration::ZERO),
            Duration::from_secs(2)
        );
        assert_eq!(
            bo.next_retry_delay(2, Duration::ZERO),
            Duration::from_secs(4)
        );
        // Eventually clamped to max.
        assert_eq!(
            bo.next_retry_delay(20, Duration::ZERO),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn jittered_backoff_bounds() {
        let bo = BackoffStrategy::ExponentialJitter {
            base: Duration::from_secs(1),
            factor: 2.0,
            max: Duration::from_secs(300),
        };
        // For attempt=3 deterministic delay is 8s. Jitter multiplier is
        // [0.5,1.5), so each sample must land in [4s, 12s].
        for _ in 0..64 {
            let d = bo.next_retry_delay(3, Duration::ZERO);
            assert!(
                d >= Duration::from_millis(4_000) && d < Duration::from_millis(12_000),
                "jittered delay out of expected bounds: {d:?}"
            );
        }
    }

    #[test]
    fn fixed_backoff_returns_fallback() {
        let bo = BackoffStrategy::Fixed;
        assert_eq!(
            bo.next_retry_delay(5, Duration::from_secs(7)),
            Duration::from_secs(7)
        );
    }

    #[test]
    fn retry_policy_default_is_jittered() {
        let rp = RetryPolicy::default();
        match rp.backoff {
            BackoffStrategy::ExponentialJitter { .. } => {}
            other => panic!("expected jittered backoff by default, got {other:?}"),
        }
    }
}
