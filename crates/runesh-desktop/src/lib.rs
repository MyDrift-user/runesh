//! Cross-platform remote desktop sharing.
//!
//! Provides screen capture, frame encoding, input injection, and clipboard
//! sharing across Windows, macOS, and Linux (X11 + Wayland architecture).
//!
//! # Features
//!
//! - **Screen Capture**: DXGI (Windows), CoreGraphics (macOS), XShm/XGetImage (X11)
//! - **Encoding**: JPEG, PNG, Zstd-compressed raw, with quality presets
//! - **Input Injection**: SendInput (Windows), CGEvent (macOS), XTest (X11)
//! - **Clipboard Sharing**: Bidirectional text clipboard sync
//! - **Multi-Monitor**: Display enumeration and per-display capture
//!
//! # Quick Start
//!
//! ```ignore
//! use axum::{Router, routing::get};
//! use runesh_desktop::{DesktopState, handlers};
//! use runesh_desktop::session::DesktopConfig;
//!
//! let state = DesktopState::new(DesktopConfig::default());
//!
//! let app = Router::new()
//!     .route("/ws/desktop", get(handlers::ws_desktop_handler))
//!     .with_state(state);
//! ```

pub mod error;
pub mod protocol;
pub mod display;
pub mod capture;
pub mod encode;
pub mod input;
pub mod cursor;
pub mod session;

#[cfg(feature = "clipboard")]
pub mod clipboard;

#[cfg(feature = "axum")]
pub mod handlers;

pub use error::DesktopError;
pub use session::DesktopConfig;

#[cfg(feature = "axum")]
pub use handlers::DesktopState;
