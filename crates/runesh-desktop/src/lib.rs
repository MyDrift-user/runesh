//! WebRTC-based cross-platform remote desktop.
//!
//! Replaces the old JSON-frame-over-WebSocket pipeline with a proper WebRTC
//! peer connection per viewer:
//!
//! - **Video**: H.264 (software via OpenH264) on an RTP track. Adaptive bitrate
//!   comes for free from the `webrtc` crate's bandwidth estimation.
//! - **Audio**: Opus on an RTP track. `cpal` captures the default output
//!   (loopback on Windows) or input device.
//! - **Control + Input**: a single ordered DataChannel carries JSON control
//!   messages *and* compact binary input frames (mouse, keyboard, scroll).
//! - **Signaling**: a WebSocket endpoint exchanges SDP and trickled ICE.
//!
//! # Quick start
//!
//! ```ignore
//! use std::sync::Arc;
//! use axum::{Router, routing::get};
//! use runesh_desktop::{DesktopConfig, handlers::{DesktopState, ws_desktop_handler}};
//! use runesh_desktop::auth::{DenyAllAuth, AlwaysDeny};
//!
//! let state = DesktopState::new(
//!     DesktopConfig::default(),
//!     Arc::new(DenyAllAuth), // replace with your real auth backend
//!     Arc::new(AlwaysDeny),
//! );
//! let app = Router::new()
//!     .route("/ws/desktop", get(ws_desktop_handler))
//!     .with_state(state);
//! ```

pub mod auth;
pub mod capture;
pub mod clipboard;
pub mod cursor;
pub mod display;
pub mod encode;
pub mod error;
pub mod input;
pub mod protocol;
pub mod session;
pub mod transport;

#[cfg(feature = "axum")]
pub mod handlers;

pub use auth::{
    AlwaysDeny, AuthError, ConsentBroker, DenyAllAuth, DesktopAuth, Operation, Principal,
};
#[cfg(any(test, feature = "insecure-test-auth"))]
pub use auth::InsecureAllowAllAuth;
pub use error::DesktopError;
pub use session::DesktopConfig;

#[cfg(feature = "axum")]
pub use handlers::DesktopState;
