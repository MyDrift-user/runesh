//! WebRTC peer connection: one instance per viewer.
//!
//! Wraps `webrtc-rs` in a small, owned API so the session manager never has
//! to touch `webrtc::…` types directly. Each [`PeerHandle`] owns:
//!
//! - One H.264 video RTP track (optional — configured at build time).
//! - One Opus audio RTP track (optional).
//! - One reliable, ordered DataChannel called `"control"` carrying JSON
//!   [`ControlRequest`] messages *and* raw binary [`input_binary`] frames.
//!
//! Callers produce an `Offer` SDP, forward it to the client over the signaling
//! WebSocket, then feed back the client's SDP answer and any trickled ICE
//! candidates. Once the peer connection state reaches `Connected`, the
//! returned `MessageReceiver` starts yielding incoming DataChannel messages.

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{Mutex, mpsc};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine};
use webrtc::api::{API, APIBuilder};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::encode::VideoSample;
#[cfg(feature = "audio")]
use crate::encode::opus_enc::AudioSample;
use crate::error::DesktopError;
use crate::transport::signal::IceServers;

/// ICE candidate received from the peer to add to this connection.
#[derive(Debug, Clone)]
pub struct RemoteIceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

/// ICE candidate the local peer wants us to send to the remote side.
#[derive(Debug, Clone)]
pub struct PeerIceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}

/// Events published by the peer for the owner to react to.
#[derive(Debug)]
pub enum PeerEvent {
    /// A locally-gathered ICE candidate ready to send to the remote peer.
    IceCandidate(PeerIceCandidate),
    /// Peer connection reached the `Connected` state.
    Connected,
    /// Peer connection closed / failed.
    Closed(String),
    /// Inbound JSON control message from the DataChannel.
    ControlJson(String),
    /// Inbound binary input frame from the DataChannel.
    InputBinary(Bytes),
}

pub struct PeerBuilder {
    ice_servers: IceServers,
    enable_video: bool,
    enable_audio: bool,
}

impl Default for PeerBuilder {
    fn default() -> Self {
        Self {
            ice_servers: IceServers::google_stun(),
            enable_video: true,
            enable_audio: false,
        }
    }
}

impl PeerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ice_servers(mut self, servers: IceServers) -> Self {
        self.ice_servers = servers;
        self
    }

    pub fn with_audio(mut self, yes: bool) -> Self {
        self.enable_audio = yes;
        self
    }

    pub fn with_video(mut self, yes: bool) -> Self {
        self.enable_video = yes;
        self
    }

    /// Construct the peer. Returns a handle plus an event receiver.
    pub async fn build(self) -> Result<(PeerHandle, mpsc::Receiver<PeerEvent>), DesktopError> {
        let api = build_api()?;
        let config = RTCConfiguration {
            ice_servers: self.ice_servers.into_webrtc(),
            ..Default::default()
        };
        let pc = Arc::new(
            api.new_peer_connection(config)
                .await
                .map_err(|e| DesktopError::Internal(format!("new_peer_connection: {e}")))?,
        );

        let (event_tx, event_rx) = mpsc::channel::<PeerEvent>(128);

        // Wire up ICE candidate gathering.
        {
            let tx = event_tx.clone();
            pc.on_ice_candidate(Box::new(move |cand: Option<RTCIceCandidate>| {
                let tx = tx.clone();
                Box::pin(async move {
                    if let Some(c) = cand {
                        match c.to_json() {
                            Ok(init) => {
                                let _ = tx
                                    .send(PeerEvent::IceCandidate(PeerIceCandidate {
                                        candidate: init.candidate,
                                        sdp_mid: init.sdp_mid,
                                        sdp_mline_index: init.sdp_mline_index,
                                    }))
                                    .await;
                            }
                            Err(e) => tracing::warn!(error = %e, "ice candidate to_json failed"),
                        }
                    }
                })
            }));
        }

        // Connection state transitions → Connected / Closed events.
        //
        // Note: `Disconnected` is *transient* — WebRTC often recovers from it
        // back into `Connected` after a brief network blip. We only treat
        // `Failed` and `Closed` as terminal.
        {
            let tx = event_tx.clone();
            pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
                let tx = tx.clone();
                Box::pin(async move {
                    match s {
                        RTCPeerConnectionState::Connected => {
                            let _ = tx.send(PeerEvent::Connected).await;
                        }
                        RTCPeerConnectionState::Disconnected => {
                            tracing::warn!("peer connection disconnected (transient)");
                        }
                        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                            let _ = tx.send(PeerEvent::Closed(format!("{s:?}"))).await;
                        }
                        _ => {}
                    }
                })
            }));
        }

        // Create the video track (if requested).
        let video_track = if self.enable_video {
            let t = Arc::new(TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line:
                        "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                            .into(),
                    rtcp_feedback: vec![],
                },
                "video".to_owned(),
                "runesh-desktop".to_owned(),
            ));
            let _sender: Arc<RTCRtpSender> = pc
                .add_track(Arc::clone(&t) as Arc<dyn TrackLocal + Send + Sync>)
                .await
                .map_err(|e| DesktopError::Internal(format!("add video track: {e}")))?;
            Some(t)
        } else {
            None
        };

        // Create the audio track (if requested).
        let audio_track = if self.enable_audio {
            let t = Arc::new(TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48_000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
                    rtcp_feedback: vec![],
                },
                "audio".to_owned(),
                "runesh-desktop".to_owned(),
            ));
            let _sender: Arc<RTCRtpSender> = pc
                .add_track(Arc::clone(&t) as Arc<dyn TrackLocal + Send + Sync>)
                .await
                .map_err(|e| DesktopError::Internal(format!("add audio track: {e}")))?;
            Some(t)
        } else {
            None
        };

        // Create the control DataChannel.
        let dc = pc
            .create_data_channel(
                "control",
                Some(RTCDataChannelInit {
                    ordered: Some(true),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| DesktopError::Internal(format!("create_data_channel: {e}")))?;

        // Forward DataChannel messages.
        {
            let tx = event_tx.clone();
            dc.on_message(Box::new(move |msg: DataChannelMessage| {
                let tx = tx.clone();
                Box::pin(async move {
                    if msg.is_string {
                        match std::str::from_utf8(&msg.data) {
                            Ok(s) => {
                                let _ = tx.send(PeerEvent::ControlJson(s.to_owned())).await;
                            }
                            Err(_) => {
                                tracing::warn!("DataChannel text was not valid UTF-8; dropping");
                            }
                        }
                    } else {
                        let _ = tx.send(PeerEvent::InputBinary(msg.data)).await;
                    }
                })
            }));
        }

        let handle = PeerHandle {
            pc,
            video_track,
            audio_track,
            dc: Arc::new(Mutex::new(dc)),
        };

        Ok((handle, event_rx))
    }
}

/// Opaque handle to an in-flight peer connection.
pub struct PeerHandle {
    pc: Arc<RTCPeerConnection>,
    video_track: Option<Arc<TrackLocalStaticSample>>,
    audio_track: Option<Arc<TrackLocalStaticSample>>,
    dc: Arc<Mutex<Arc<RTCDataChannel>>>,
}

impl PeerHandle {
    /// Create the server's SDP offer.
    pub async fn create_offer(&self) -> Result<String, DesktopError> {
        let offer = self
            .pc
            .create_offer(None)
            .await
            .map_err(|e| DesktopError::Internal(format!("create_offer: {e}")))?;
        self.pc
            .set_local_description(offer.clone())
            .await
            .map_err(|e| DesktopError::Internal(format!("set_local_description: {e}")))?;
        Ok(offer.sdp)
    }

    /// Accept the client's SDP answer.
    pub async fn set_remote_answer(&self, sdp: String) -> Result<(), DesktopError> {
        let answer = RTCSessionDescription::answer(sdp)
            .map_err(|e| DesktopError::Internal(format!("parse answer sdp: {e}")))?;
        self.pc
            .set_remote_description(answer)
            .await
            .map_err(|e| DesktopError::Internal(format!("set_remote_description: {e}")))?;
        Ok(())
    }

    /// Add a trickled ICE candidate from the client.
    pub async fn add_remote_ice(&self, cand: RemoteIceCandidate) -> Result<(), DesktopError> {
        let init = RTCIceCandidateInit {
            candidate: cand.candidate,
            sdp_mid: cand.sdp_mid,
            sdp_mline_index: cand.sdp_mline_index,
            username_fragment: None,
        };
        self.pc
            .add_ice_candidate(init)
            .await
            .map_err(|e| DesktopError::Internal(format!("add_ice_candidate: {e}")))?;
        Ok(())
    }

    /// Push an encoded H.264 sample to the video track.
    pub async fn send_video(&self, sample: &VideoSample) -> Result<(), DesktopError> {
        let Some(track) = &self.video_track else {
            return Err(DesktopError::Internal("peer has no video track".into()));
        };
        track
            .write_sample(&Sample {
                data: Bytes::copy_from_slice(&sample.data),
                duration: sample.duration,
                ..Default::default()
            })
            .await
            .map_err(|e| DesktopError::Internal(format!("write_sample video: {e}")))?;
        Ok(())
    }

    /// Push an encoded Opus packet to the audio track.
    #[cfg(feature = "audio")]
    pub async fn send_audio(&self, sample: &AudioSample) -> Result<(), DesktopError> {
        let Some(track) = &self.audio_track else {
            return Err(DesktopError::Internal("peer has no audio track".into()));
        };
        track
            .write_sample(&Sample {
                data: Bytes::copy_from_slice(&sample.data),
                duration: sample.duration,
                ..Default::default()
            })
            .await
            .map_err(|e| DesktopError::Internal(format!("write_sample audio: {e}")))?;
        Ok(())
    }

    /// Send a JSON control message over the DataChannel.
    pub async fn send_control_json(&self, json: &str) -> Result<(), DesktopError> {
        let dc = self.dc.lock().await;
        dc.send_text(json.to_owned())
            .await
            .map_err(|e| DesktopError::Internal(format!("send_text: {e}")))?;
        Ok(())
    }

    /// Close the peer connection.
    pub async fn close(&self) -> Result<(), DesktopError> {
        self.pc
            .close()
            .await
            .map_err(|e| DesktopError::Internal(format!("pc close: {e}")))?;
        Ok(())
    }

    pub fn has_video_track(&self) -> bool {
        self.video_track.is_some()
    }
    pub fn has_audio_track(&self) -> bool {
        self.audio_track.is_some()
    }
}

/// Build a `webrtc::API` with H.264 + Opus registered and the default
/// interceptors (NACK, report generators) wired up.
fn build_api() -> Result<API, DesktopError> {
    let mut m = MediaEngine::default();

    // H.264 — baseline packetization-mode=1 for maximum decoder compatibility.
    m.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".into(),
                rtcp_feedback: vec![],
            },
            payload_type: 102,
            ..Default::default()
        },
        RTPCodecType::Video,
    )
    .map_err(|e| DesktopError::Internal(format!("register H264 codec: {e}")))?;

    // Opus 48 kHz stereo.
    m.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48_000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
                rtcp_feedback: vec![],
            },
            payload_type: 111,
            ..Default::default()
        },
        RTPCodecType::Audio,
    )
    .map_err(|e| DesktopError::Internal(format!("register Opus codec: {e}")))?;

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)
        .map_err(|e| DesktopError::Internal(format!("register interceptors: {e}")))?;

    Ok(APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build())
}
