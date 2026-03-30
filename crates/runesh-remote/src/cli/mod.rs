//! Remote CLI module: PTY-based terminal sessions over WebSocket.

pub mod audit;

#[cfg(feature = "cli")]
pub mod pty;

#[cfg(feature = "cli")]
pub mod session;

pub use audit::AuditLogger;

#[cfg(feature = "cli")]
pub use pty::PtyHandle;

#[cfg(feature = "cli")]
pub use session::{SessionConfig, SessionManager};
