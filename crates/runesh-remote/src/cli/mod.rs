//! Remote CLI module: PTY-based terminal sessions over WebSocket.

pub mod audit;

#[cfg(feature = "cli")]
pub mod pty;

#[cfg(all(feature = "cli", windows))]
pub mod pty_as_user;

#[cfg(feature = "cli")]
pub mod session;

pub use audit::AuditLogger;

#[cfg(feature = "cli")]
pub use pty::PtyHandle;

#[cfg(all(feature = "cli", windows))]
pub use pty_as_user::{
    PtyAsUserHandle, PtyReader, PtyWriter, spawn_as_active_user, spawn_with_credentials,
};

#[cfg(feature = "cli")]
pub use session::{SessionConfig, SessionManager};
