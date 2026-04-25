#![deny(unsafe_code)]
//! RDP client for runesh consumers.
//!
//! Wraps the [IronRDP](https://github.com/Devolutions/IronRDP) crate
//! suite into the same shape as [`runesh_desktop`]'s capture +
//! encoder pipeline so a remote-desktop view of an RDP-only target
//! plugs into the existing WebRTC peer connection without any
//! protocol-specific code in the consumer.
//!
//! Typical use:
//!
//! ```ignore
//! use runesh_rdp::{RdpLogonParams, RdpSession};
//! use secrecy::SecretString;
//!
//! let params = RdpLogonParams {
//!     host: "127.0.0.1".into(),
//!     port: 3389,
//!     username: "Administrator".into(),
//!     password: SecretString::new("hunter2".into()),
//!     domain: None,
//!     width: 1920,
//!     height: 1080,
//!     fps_target: 15,
//!     ignore_cert: true,
//! };
//! let mut session = RdpSession::connect(params).await?;
//! while let Some(sample) = session.next_sample().await {
//!     peer.send_video(sample?).await?;
//! }
//! ```
//!
//! Shape mirrors [`runesh_desktop::transport::webrtc_peer::PeerHandle`]
//! so swapping in an RDP source on top of the existing capture loop
//! is mechanical: replace the `ScreenCapture + VideoEncoder` pair
//! with `RdpSession::next_sample`. The session runs IronRDP's frame
//! loop on a background tokio task; the foreground driver only sees
//! ready [`VideoSample`]s and forwards [`InputEvent`]s.
//!
//! This crate is a v1: it compiles against IronRDP 0.14 / ironrdp-tokio
//! 0.8 / ironrdp-tls 0.2. NLA / CredSSP edge cases (Active Directory
//! domains, FIPS, smart card) and bitmap-update edge cases (mixed
//! tile sizes, incremental updates with rect coalescing) will need
//! live-target iteration; they're not testable in CI.

mod error;
mod input;
mod precondition;
mod session;

pub use error::RdpError;
pub use input::InputEvent;
pub use precondition::rdp_enabled;
pub use session::{RdpLogonParams, RdpSession};

// Re-export VideoSample so consumers don't have to depend on
// runesh-desktop just to receive the frames we produce.
pub use runesh_desktop::encode::VideoSample;
