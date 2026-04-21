#![deny(unsafe_code)]
//! Monitoring check engine with alert state machine.
//!
//! Runs health probes (HTTP, TCP, process, disk, custom) on a schedule,
//! tracks alert state transitions with flap prevention, and emits events
//! for notification dispatch.

pub mod alert;
pub mod check;
pub mod scheduler;

pub use alert::{
    Alert, AlertManager, AlertSnapshot, AlertState, AlertStore, AlertStoreError, FileAlertStore,
    InMemoryAlertStore,
};
pub use check::{
    Check, CheckResult, CheckRuntime, CheckStatus, CheckType, CommandCheckPolicy, HttpCheckPolicy,
};
pub use scheduler::CheckScheduler;
