//! Transport layer for remote desktop sessions.
//!
//! - [`signal`] — signaling messages carried over the WebSocket (SDP, ICE).
//! - [`webrtc_peer`] — one WebRTC peer connection per viewer, with H.264 video
//!   RTP track, Opus audio RTP track, and a reliable DataChannel for control
//!   + binary input.

pub mod signal;

#[cfg(feature = "webrtc-transport")]
pub mod webrtc_peer;

#[cfg(feature = "webrtc-transport")]
pub use webrtc_peer::{PeerBuilder, PeerHandle, PeerIceCandidate, RemoteIceCandidate};
