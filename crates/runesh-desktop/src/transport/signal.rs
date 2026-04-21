//! WebSocket-carried signaling helpers.
//!
//! The wire format lives in [`crate::protocol::SignalRequest`] /
//! [`crate::protocol::SignalResponse`]. This module just provides a handful
//! of constructors and an [`IceServers`] convenience type so the session
//! manager does not have to reach into `webrtc` types directly.

use serde::{Deserialize, Serialize};

/// A STUN / TURN server passed to [`webrtc::peer_connection::configuration::RTCConfiguration`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// ICE configuration for the peer connection.
#[derive(Debug, Clone, Default)]
pub struct IceServers(pub Vec<IceServer>);

impl IceServers {
    /// Google's public STUN server — sensible default for testing.
    pub fn google_stun() -> Self {
        Self(vec![IceServer {
            urls: vec!["stun:stun.l.google.com:19302".into()],
            username: None,
            credential: None,
        }])
    }
}

#[cfg(feature = "webrtc-transport")]
impl IceServers {
    pub fn into_webrtc(self) -> Vec<webrtc::ice_transport::ice_server::RTCIceServer> {
        self.0
            .into_iter()
            .map(|s| webrtc::ice_transport::ice_server::RTCIceServer {
                urls: s.urls,
                username: s.username.unwrap_or_default(),
                credential: s.credential.unwrap_or_default(),
            })
            .collect()
    }
}
