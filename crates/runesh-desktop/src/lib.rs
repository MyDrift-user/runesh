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

pub mod auth;
pub mod capture;
pub mod cursor;
pub mod display;
pub mod encode;
pub mod error;
pub mod input;
pub mod protocol;
pub mod session;

// Clipboard module is always compiled (for direction/settings types).
// The platform `ClipboardManager` backend is still feature-gated internally.
pub mod clipboard;

#[cfg(feature = "axum")]
pub mod handlers;

pub use auth::{
    AllowAllAuth, AlwaysDeny, AuthError, ConsentBroker, DenyAllAuth, DesktopAuth, Operation,
    Principal,
};
pub use error::DesktopError;
pub use session::DesktopConfig;

#[cfg(feature = "axum")]
pub use handlers::DesktopState;
