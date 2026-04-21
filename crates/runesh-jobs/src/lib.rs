#![deny(unsafe_code)]
pub mod executor;
pub mod task;

pub use executor::{
    CommandAllowlist, Executor, NullVerifier, RejectScripts, TaskVerifier, VerifyError,
    execute_command, execute_task, execute_task_with_policy,
};
pub use task::{
    AgentTask, BackoffStrategy, RetryPolicy, TaskQueue, TaskResult, TaskStatus, TaskType,
};
