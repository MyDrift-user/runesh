//! Remote file explorer and CLI over WebSocket.
//!
//! Provides secure, cross-platform remote access to file systems and terminal
//! sessions. Designed for enterprise use with path traversal prevention,
//! audit logging, and configurable security policies.
//!
//! # Features
//!
//! - **File Explorer**: List, read, write, copy, move, delete, search, archive
//! - **Remote CLI**: PTY-based terminal sessions (ConPTY on Windows, Unix PTY on Linux/macOS)
//! - **Security**: Path sandboxing, operation allowlists, audit logging
//! - **Chunked Transfer**: Large file upload/download with progress tracking
//!
//! # Quick Start
//!
//! ```ignore
//! use axum::{Router, routing::get};
//! use runesh_remote::{RemoteState, handlers};
//! use runesh_remote::fs::FsPolicy;
//! use runesh_remote::cli::SessionConfig;
//!
//! let state = RemoteState::new(
//!     FsPolicy::default(),
//!     SessionConfig::default(),
//! );
//!
//! let app = Router::new()
//!     .route("/ws/remote", get(handlers::ws_remote_handler))
//!     .with_state(state);
//! ```

pub mod error;
pub mod protocol;

#[cfg(feature = "fs")]
pub mod fs;

pub mod cli;

#[cfg(feature = "axum")]
pub mod handlers;

pub use error::RemoteError;

#[cfg(feature = "axum")]
pub use handlers::RemoteState;
