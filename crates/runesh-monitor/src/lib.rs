//! Monitoring check engine with alert state machine.
//!
//! Runs health probes (HTTP, TCP, process, disk, custom) on a schedule,
//! tracks alert state transitions with flap prevention, and emits events
//! for notification dispatch.

pub mod alert;
pub mod check;

pub use alert::{Alert, AlertManager, AlertState};
pub use check::{Check, CheckResult, CheckStatus, CheckType};
