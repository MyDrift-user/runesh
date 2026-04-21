//! Axum WebSocket handler for WebRTC signaling.
//!
//! The WebSocket is **signaling only**: auth, SDP offer/answer, trickled ICE.
//! Once the peer connection is `Connected`, video goes on the H.264 RTP track,
//! audio on the Opus RTP track, and control / input on a single DataChannel.

#[cfg(all(feature = "axum", feature = "webrtc-transport"))]
mod axum_handlers {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::State;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use futures_util::stream::{SplitSink, SplitStream};
    use futures_util::{SinkExt, StreamExt};
    use tokio::sync::mpsc;

    use crate::auth::{ConsentBroker, DesktopAuth, Operation, Principal};
    use crate::input;
    use crate::protocol::input_binary::{InputEvent, decode as decode_binary_input};
    use crate::protocol::*;
    use crate::session::{DesktopConfig, DesktopSessionManager, VideoPipeline};
    use crate::transport::signal::IceServers;
    use crate::transport::webrtc_peer::PeerEvent;
    use crate::transport::{PeerBuilder, PeerHandle, PeerIceCandidate, RemoteIceCandidate};

    const MAX_FRAME_SIZE: usize = 1 << 20;
    const MAX_MESSAGE_SIZE: usize = 4 << 20;
    const AUTH_DEADLINE: Duration = Duration::from_secs(5);
    const START_DEADLINE: Duration = Duration::from_secs(30);
    /// If the peer doesn't reach `Connected` within this window after the
    /// offer is sent, we close the peer and stop the capture pipeline so it
    /// doesn't burn CPU indefinitely.
    const PEER_CONNECT_DEADLINE: Duration = Duration::from_secs(45);

    type WsSink = SplitSink<WebSocket, Message>;
    type WsStream = SplitStream<WebSocket>;

    /// Shared state for desktop WebSocket handlers.
    #[derive(Clone)]
    pub struct DesktopState {
        pub session_manager: Arc<DesktopSessionManager>,
        pub auth: Arc<dyn DesktopAuth>,
        pub consent: Arc<dyn ConsentBroker>,
        pub ice_servers: IceServers,
    }

    impl DesktopState {
        pub fn new(
            config: DesktopConfig,
            auth: Arc<dyn DesktopAuth>,
            consent: Arc<dyn ConsentBroker>,
        ) -> Self {
            Self {
                session_manager: Arc::new(DesktopSessionManager::new(config)),
                auth,
                consent,
                ice_servers: IceServers::google_stun(),
            }
        }

        pub fn with_ice_servers(mut self, servers: IceServers) -> Self {
            self.ice_servers = servers;
            self
        }
    }

    pub async fn ws_desktop_handler(
        ws: WebSocketUpgrade,
        State(state): State<DesktopState>,
    ) -> impl IntoResponse {
        ws.max_frame_size(MAX_FRAME_SIZE)
            .max_message_size(MAX_MESSAGE_SIZE)
            .on_upgrade(move |socket| handle_desktop_ws(socket, state))
    }

    fn err_frame(code: &str, msg: &str) -> Message {
        Message::Text(
            serde_json::to_string(&SignalResponse::Error {
                code: code.into(),
                message: msg.into(),
            })
            .unwrap_or_default()
            .into(),
        )
    }

    async fn send_signal(ws_tx: &mut WsSink, resp: &SignalResponse) -> Result<(), ()> {
        ws_tx
            .send(Message::Text(
                serde_json::to_string(resp).unwrap_or_default().into(),
            ))
            .await
            .map_err(|_| ())
    }

    async fn handle_desktop_ws(socket: WebSocket, state: DesktopState) {
        let (mut ws_tx, mut ws_rx) = socket.split();

        // ── 1. Authenticate ──────────────────────────────────────────────
        let principal = match auth_step(&mut ws_tx, &mut ws_rx, &state).await {
            Some(p) => p,
            None => return,
        };

        // ── 2. Register cursor for this user ─────────────────────────────
        let cursor_id = uuid::Uuid::new_v4().to_string();
        let cursor_color = register_cursor(&state, &cursor_id, &principal).await;

        if send_signal(
            &mut ws_tx,
            &SignalResponse::AuthOk {
                cursor_id: cursor_id.clone(),
                cursor_color,
            },
        )
        .await
        .is_err()
        {
            cleanup_cursor(&state, &cursor_id).await;
            return;
        }

        // ── 3. Wait for Start, spin up capture + peer, emit Offer ────────
        let started = start_step(&mut ws_tx, &mut ws_rx, &state, &principal).await;
        let (peer, mut peer_events, session_id, pipeline, audio_on) = match started {
            Some(t) => t,
            None => {
                cleanup_cursor(&state, &cursor_id).await;
                return;
            }
        };
        let peer = Arc::new(peer);

        // ── 4. Input consent (once) ──────────────────────────────────────
        let input_consent = state
            .consent
            .request_input_consent(&session_id, &principal)
            .await;
        let injector: Option<Box<dyn input::InputInjector>> =
            if input_consent && state.session_manager.allow_input() {
                input::create_injector().ok()
            } else {
                None
            };
        let injector = Arc::new(tokio::sync::Mutex::new(injector));

        // ── 5. State carried through the main loop ──────────────────────
        //
        // `current_pipeline` is swappable: SelectDisplay replaces it.
        // `connected` flips to true on PeerEvent::Connected; forwarders and
        // cursor broadcaster only start once connected, to avoid sending on
        // a DataChannel that isn't open yet.
        let current_pipeline = Arc::new(tokio::sync::Mutex::new(Arc::clone(&pipeline)));
        let mut video_forwarder: Option<tokio::task::JoinHandle<()>> = None;
        let mut cursor_broadcaster: Option<tokio::task::JoinHandle<()>> = None;
        #[cfg(feature = "audio")]
        let mut audio_forwarder: Option<tokio::task::JoinHandle<()>> = None;
        #[cfg(not(feature = "audio"))]
        let _ = audio_on;

        let mut connected = false;
        let connect_deadline = tokio::time::sleep(PEER_CONNECT_DEADLINE);
        tokio::pin!(connect_deadline);

        // ── 6. Main signaling / event loop ───────────────────────────────
        let mut clip_writes: std::collections::VecDeque<Instant> =
            std::collections::VecDeque::new();

        loop {
            tokio::select! {
                biased;

                // Peer-connect timeout: only armed before we reach Connected.
                _ = &mut connect_deadline, if !connected => {
                    tracing::warn!(session_id = %session_id, "peer did not connect within deadline, closing");
                    let _ = send_signal(&mut ws_tx, &SignalResponse::PeerClosed {
                        reason: "connect_timeout".into(),
                    }).await;
                    break;
                }

                ws_msg = ws_rx.next() => {
                    let text = match ws_msg {
                        Some(Ok(Message::Text(t))) => t.to_string(),
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => continue,
                    };
                    let req: SignalRequest = match serde_json::from_str(&text) {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = ws_tx.send(err_frame("parse_error", &e.to_string())).await;
                            continue;
                        }
                    };
                    match req {
                        SignalRequest::Answer { sdp } => {
                            if let Err(e) = peer.set_remote_answer(sdp).await {
                                let _ = ws_tx.send(err_frame(e.error_code(), &e.to_string())).await;
                            }
                        }
                        SignalRequest::IceCandidate { candidate, sdp_mid, sdp_mline_index } => {
                            let cand = RemoteIceCandidate { candidate, sdp_mid, sdp_mline_index };
                            if let Err(e) = peer.add_remote_ice(cand).await {
                                tracing::warn!(error = %e, "add_remote_ice failed");
                            }
                        }
                        SignalRequest::ListDisplays => {
                            match crate::display::enumerate_displays() {
                                Ok(displays) => {
                                    let _ = send_signal(&mut ws_tx, &SignalResponse::Displays { displays }).await;
                                }
                                Err(e) => {
                                    let _ = ws_tx.send(err_frame(e.error_code(), &e.to_string())).await;
                                }
                            }
                        }
                        SignalRequest::Hangup => {
                            let _ = peer.close().await;
                            break;
                        }
                        SignalRequest::Auth { .. } | SignalRequest::Start { .. } => {
                            let _ = ws_tx.send(err_frame("protocol_error", "already started")).await;
                        }
                    }
                }

                ev = peer_events.recv() => {
                    let Some(ev) = ev else { break };
                    match ev {
                        PeerEvent::IceCandidate(PeerIceCandidate { candidate, sdp_mid, sdp_mline_index }) => {
                            let _ = send_signal(&mut ws_tx, &SignalResponse::IceCandidate {
                                candidate, sdp_mid, sdp_mline_index,
                            }).await;
                        }
                        PeerEvent::Connected => {
                            if connected {
                                // Stale re-entry (e.g. Disconnected → Connected bounce).
                                continue;
                            }
                            connected = true;
                            let _ = send_signal(&mut ws_tx, &SignalResponse::PeerConnected {
                                session_id: session_id.clone(),
                            }).await;
                            // The DataChannel is open now — spawn the forwarders.
                            video_forwarder = Some(spawn_video_forwarder(
                                Arc::clone(&peer),
                                Arc::clone(&current_pipeline),
                            ));
                            cursor_broadcaster = Some(spawn_cursor_broadcaster(
                                Arc::clone(&peer),
                                Arc::clone(state.session_manager.cursor_tracker()),
                                state.session_manager.multi_cursor_enabled(),
                            ));
                            #[cfg(feature = "audio")]
                            {
                                if audio_on && state.session_manager.audio_enabled() {
                                    if let Ok(ap) = state.session_manager.ensure_audio_pipeline().await {
                                        audio_forwarder = Some(spawn_audio_forwarder(Arc::clone(&peer), ap));
                                    }
                                }
                            }
                            // Nudge the encoder for an IDR so the client can decode immediately.
                            current_pipeline.lock().await.request_keyframe();
                        }
                        PeerEvent::Closed(reason) => {
                            let _ = send_signal(&mut ws_tx, &SignalResponse::PeerClosed { reason }).await;
                            break;
                        }
                        PeerEvent::ControlJson(json) => {
                            handle_control_json(
                                &peer,
                                &state,
                                &principal,
                                Arc::clone(&current_pipeline),
                                &session_id,
                                &mut video_forwarder,
                                &json,
                                &mut clip_writes,
                            ).await;
                        }
                        PeerEvent::InputBinary(bytes) => {
                            handle_input_binary(&state, &principal, &cursor_id, &bytes, &injector, input_consent).await;
                        }
                    }
                }
            }
        }

        // Cleanup: stop forwarders before dropping the peer so their last
        // write_sample attempts don't race with shutdown.
        if let Some(h) = video_forwarder {
            h.abort();
        }
        if let Some(h) = cursor_broadcaster {
            h.abort();
        }
        #[cfg(feature = "audio")]
        if let Some(h) = audio_forwarder {
            h.abort();
        }

        let _ = peer.close().await;
        let _ = state.session_manager.stop_session(&session_id).await;
        cleanup_cursor(&state, &cursor_id).await;
        tracing::info!(session_id = %session_id, "desktop ws closed");
    }

    // ── Forwarder spawners ────────────────────────────────────────────────

    /// Drain the pipeline's broadcast and write samples to the peer's track.
    /// On [`RecvError::Lagged`] ask the pipeline for an IDR so the client can
    /// resync instead of waiting for the periodic keyframe.
    fn spawn_video_forwarder(
        peer: Arc<PeerHandle>,
        pipeline: Arc<tokio::sync::Mutex<Arc<crate::session::VideoPipeline>>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = pipeline.lock().await.subscribe();
            loop {
                match rx.recv().await {
                    Ok(sample) => {
                        if peer.send_video(&sample).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(dropped = n, "video viewer lagged, forcing keyframe");
                        pipeline.lock().await.request_keyframe();
                        // `rx` is now re-synced (broadcast resumes at latest).
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Pipeline tore down; try to rebind in case of a display switch.
                        let new_rx = pipeline.lock().await.subscribe();
                        rx = new_rx;
                    }
                }
            }
        })
    }

    fn spawn_cursor_broadcaster(
        peer: Arc<PeerHandle>,
        tracker: Arc<tokio::sync::RwLock<crate::cursor::CursorTracker>>,
        enabled: bool,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if !enabled {
                return;
            }
            let mut ticker = tokio::time::interval(Duration::from_millis(50));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let cursors = tracker.read().await.all_cursors();
                if cursors.is_empty() {
                    continue;
                }
                let msg = ControlResponse::CursorPositions { cursors };
                let json = match serde_json::to_string(&msg) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if peer.send_control_json(&json).await.is_err() {
                    break;
                }
            }
        })
    }

    /// Swap the video pipeline under the forwarder. Used by both quality
    /// changes and display switches. Aborts the old forwarder, replaces the
    /// shared pipeline arc, forces an IDR so the client picks up the new
    /// SPS/PPS (dimensions or bitrate may differ), and respawns the forwarder.
    async fn swap_pipeline(
        slot: Arc<tokio::sync::Mutex<Arc<VideoPipeline>>>,
        new_pipeline: Arc<VideoPipeline>,
        video_forwarder: &mut Option<tokio::task::JoinHandle<()>>,
        peer: Arc<PeerHandle>,
        session_id: &str,
        reason: &str,
    ) {
        if let Some(h) = video_forwarder.take() {
            h.abort();
        }
        {
            let mut g = slot.lock().await;
            *g = Arc::clone(&new_pipeline);
        }
        new_pipeline.request_keyframe();
        *video_forwarder = Some(spawn_video_forwarder(peer, slot));
        tracing::info!(session_id, reason, "video pipeline swapped");
    }

    #[cfg(feature = "audio")]
    fn spawn_audio_forwarder(
        peer: Arc<PeerHandle>,
        pipeline: Arc<crate::session::AudioPipeline>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = pipeline.subscribe();
            loop {
                match rx.recv().await {
                    Ok(sample) => {
                        if peer.send_audio(&sample).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(dropped = n, "audio viewer lagged");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    // ── Steps ────────────────────────────────────────────────────────────

    async fn auth_step(
        ws_tx: &mut WsSink,
        ws_rx: &mut WsStream,
        state: &DesktopState,
    ) -> Option<Principal> {
        let text = match tokio::time::timeout(AUTH_DEADLINE, ws_rx.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => t.to_string(),
            _ => {
                let _ = ws_tx
                    .send(err_frame(
                        "unauthenticated",
                        "auth frame required within 5s",
                    ))
                    .await;
                return None;
            }
        };
        let req: SignalRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let _ = ws_tx.send(err_frame("parse_error", &e.to_string())).await;
                return None;
            }
        };
        let (token, display_name) = match req {
            SignalRequest::Auth {
                token,
                display_name,
            } => (token, display_name),
            _ => {
                let _ = ws_tx
                    .send(err_frame("unauthenticated", "first frame must be Auth"))
                    .await;
                return None;
            }
        };
        match state.auth.authenticate(&token).await {
            Ok(mut p) => {
                if p.display_name.is_none() {
                    p.display_name = display_name;
                }
                Some(p)
            }
            Err(e) => {
                let _ = ws_tx
                    .send(err_frame("unauthenticated", &e.to_string()))
                    .await;
                None
            }
        }
    }

    async fn start_step(
        ws_tx: &mut WsSink,
        ws_rx: &mut WsStream,
        state: &DesktopState,
        principal: &Principal,
    ) -> Option<(
        PeerHandle,
        mpsc::Receiver<PeerEvent>,
        String,
        Arc<VideoPipeline>,
        bool,
    )> {
        let text = match tokio::time::timeout(START_DEADLINE, ws_rx.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => t.to_string(),
            _ => {
                let _ = ws_tx
                    .send(err_frame("protocol_error", "expected Start within 30s"))
                    .await;
                return None;
            }
        };
        let req: SignalRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let _ = ws_tx.send(err_frame("parse_error", &e.to_string())).await;
                return None;
            }
        };
        let (display_id, quality, max_fps, audio) = match req {
            SignalRequest::Start {
                display_id,
                quality,
                max_fps,
                audio,
            } => (display_id, quality, max_fps, audio),
            _ => {
                let _ = ws_tx
                    .send(err_frame("protocol_error", "expected Start"))
                    .await;
                return None;
            }
        };

        if let Err(e) = state.auth.authorize(principal, &Operation::StartSession) {
            let _ = ws_tx.send(err_frame("forbidden", &e.to_string())).await;
            return None;
        }

        let (session_id, _display, pipeline) = match state
            .session_manager
            .start_session(display_id, Some(quality), Some(max_fps))
            .await
        {
            Ok(t) => t,
            Err(e) => {
                let _ = ws_tx.send(err_frame(e.error_code(), &e.to_string())).await;
                return None;
            }
        };

        let audio_on = audio && state.session_manager.audio_enabled();
        let built = PeerBuilder::new()
            .ice_servers(state.ice_servers.clone())
            .with_video(true)
            .with_audio(audio_on)
            .build()
            .await;
        let (peer, events) = match built {
            Ok(p) => p,
            Err(e) => {
                let _ = ws_tx.send(err_frame(e.error_code(), &e.to_string())).await;
                let _ = state.session_manager.stop_session(&session_id).await;
                return None;
            }
        };

        let sdp = match peer.create_offer().await {
            Ok(s) => s,
            Err(e) => {
                let _ = ws_tx.send(err_frame(e.error_code(), &e.to_string())).await;
                let _ = peer.close().await;
                let _ = state.session_manager.stop_session(&session_id).await;
                return None;
            }
        };
        if send_signal(
            ws_tx,
            &SignalResponse::Offer {
                session_id: session_id.clone(),
                sdp,
            },
        )
        .await
        .is_err()
        {
            let _ = peer.close().await;
            let _ = state.session_manager.stop_session(&session_id).await;
            return None;
        }

        Some((peer, events, session_id, pipeline, audio_on))
    }

    async fn register_cursor(
        state: &DesktopState,
        cursor_id: &str,
        principal: &Principal,
    ) -> String {
        if !state.session_manager.multi_cursor_enabled() {
            return "#E74C3C".to_owned();
        }
        let label = principal
            .display_name
            .clone()
            .unwrap_or_else(|| "Remote".to_string());
        state
            .session_manager
            .cursor_tracker()
            .write()
            .await
            .add_cursor(cursor_id, &label, false)
    }

    async fn cleanup_cursor(state: &DesktopState, cursor_id: &str) {
        if state.session_manager.multi_cursor_enabled() {
            state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .remove_cursor(cursor_id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_control_json(
        peer: &Arc<PeerHandle>,
        state: &DesktopState,
        principal: &Principal,
        pipeline: Arc<tokio::sync::Mutex<Arc<VideoPipeline>>>,
        session_id: &str,
        video_forwarder: &mut Option<tokio::task::JoinHandle<()>>,
        json: &str,
        clip_writes: &mut std::collections::VecDeque<Instant>,
    ) {
        let req: ControlRequest = match serde_json::from_str(json) {
            Ok(r) => r,
            Err(e) => {
                let _ = peer
                    .send_control_json(
                        &serde_json::to_string(&ControlResponse::Error {
                            code: "parse_error".into(),
                            message: e.to_string(),
                        })
                        .unwrap_or_default(),
                    )
                    .await;
                return;
            }
        };
        match req {
            ControlRequest::Ping { nonce } => {
                let _ = peer
                    .send_control_json(
                        &serde_json::to_string(&ControlResponse::Pong { nonce })
                            .unwrap_or_default(),
                    )
                    .await;
            }
            ControlRequest::RequestKeyFrame => {
                pipeline.lock().await.request_keyframe();
            }
            ControlRequest::SetQuality { quality } => {
                let current = pipeline.lock().await.clone();
                // Rebuild the pipeline at the new quality. openh264's Rust binding
                // cannot change bitrate at runtime, so a full swap is the only way.
                match state
                    .session_manager
                    .rebuild_video_pipeline(current.display().id, quality, None)
                    .await
                {
                    Ok(new_p) => {
                        swap_pipeline(
                            pipeline,
                            new_p,
                            video_forwarder,
                            Arc::clone(peer),
                            session_id,
                            "quality",
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "set_quality rebuild failed");
                    }
                }
            }
            ControlRequest::SelectDisplay { display_id } => {
                match state
                    .session_manager
                    .switch_display(display_id, None, None)
                    .await
                {
                    Ok(new_p) => {
                        swap_pipeline(
                            pipeline,
                            new_p,
                            video_forwarder,
                            Arc::clone(peer),
                            session_id,
                            "display",
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = peer
                            .send_control_json(
                                &serde_json::to_string(&ControlResponse::Error {
                                    code: e.error_code().into(),
                                    message: e.to_string(),
                                })
                                .unwrap_or_default(),
                            )
                            .await;
                    }
                }
            }
            ControlRequest::SetCursorMode { mode } => {
                if state.session_manager.multi_cursor_enabled() {
                    state.session_manager.cursor_tracker().write().await.mode = mode;
                }
            }
            ControlRequest::SetClipboard { content } => {
                let clip = state.session_manager.clipboard_settings();
                if !state.session_manager.allow_clipboard()
                    || !clip.direction.allows_viewer_to_host()
                {
                    let _ = peer
                        .send_control_json(
                            &serde_json::to_string(&ControlResponse::Error {
                                code: "clipboard_disabled".into(),
                                message: "viewer-to-host clipboard not allowed".into(),
                            })
                            .unwrap_or_default(),
                        )
                        .await;
                    return;
                }
                if content.len() > clip.max_bytes {
                    let _ = peer
                        .send_control_json(
                            &serde_json::to_string(&ControlResponse::Error {
                                code: "clipboard_too_large".into(),
                                message: format!("exceeds {} bytes", clip.max_bytes),
                            })
                            .unwrap_or_default(),
                        )
                        .await;
                    return;
                }
                let now = Instant::now();
                let window = Duration::from_secs(1);
                while let Some(&front) = clip_writes.front() {
                    if now.duration_since(front) > window {
                        clip_writes.pop_front();
                    } else {
                        break;
                    }
                }
                if clip_writes.len() as u32 >= clip.write_rate_per_sec {
                    let _ = peer
                        .send_control_json(
                            &serde_json::to_string(&ControlResponse::Error {
                                code: "clipboard_rate_limited".into(),
                                message: "too many writes".into(),
                            })
                            .unwrap_or_default(),
                        )
                        .await;
                    return;
                }
                clip_writes.push_back(now);
                if let Err(e) = state.auth.authorize(principal, &Operation::SetClipboard) {
                    let _ = peer
                        .send_control_json(
                            &serde_json::to_string(&ControlResponse::Error {
                                code: "forbidden".into(),
                                message: e.to_string(),
                            })
                            .unwrap_or_default(),
                        )
                        .await;
                    return;
                }
                #[cfg(feature = "clipboard")]
                {
                    if let Ok(mut cb) = crate::clipboard::ClipboardManager::new() {
                        let _ = cb.set_text(&content);
                    }
                }
                let _ = peer
                    .send_control_json(
                        &serde_json::to_string(&ControlResponse::ClipboardUpdate { content })
                            .unwrap_or_default(),
                    )
                    .await;
            }
        }
    }

    async fn handle_input_binary(
        state: &DesktopState,
        principal: &Principal,
        cursor_id: &str,
        bytes: &[u8],
        injector: &Arc<tokio::sync::Mutex<Option<Box<dyn input::InputInjector>>>>,
        input_consent: bool,
    ) {
        let event = match decode_binary_input(bytes) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(error = e, "binary input decode failed");
                return;
            }
        };

        if state.session_manager.multi_cursor_enabled() {
            let mut tracker = state.session_manager.cursor_tracker().write().await;
            match &event {
                InputEvent::MouseMove { x, y } => tracker.update_position(cursor_id, *x, *y),
                InputEvent::MouseButton { x, y, pressed, .. } if *pressed => {
                    tracker.update_position(cursor_id, *x, *y);
                    if tracker.mode == MultiCursorMode::Collaborative {
                        tracker.set_focus(cursor_id);
                    }
                }
                _ => {}
            }
        }

        if !input_consent || !state.session_manager.allow_input() {
            return;
        }
        if state
            .auth
            .authorize(principal, &Operation::InjectInput)
            .is_err()
        {
            return;
        }
        if state.session_manager.multi_cursor_enabled()
            && !state
                .session_manager
                .cursor_tracker()
                .read()
                .await
                .should_inject_input(cursor_id)
        {
            return;
        }

        let mut guard = injector.lock().await;
        let Some(inj) = guard.as_mut() else { return };
        let res = match event {
            InputEvent::MouseMove { x, y } => inj.mouse_move(x, y),
            InputEvent::MouseButton {
                button,
                pressed,
                x,
                y,
            } => inj.mouse_button(button, pressed, x, y),
            InputEvent::KeyEvent {
                key_code,
                pressed,
                modifiers,
            } => inj.key_event(key_code, pressed, modifiers),
            InputEvent::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => inj.scroll(x, y, delta_x, delta_y),
        };
        if let Err(e) = res {
            tracing::warn!(error = %e, "input injection failed");
        }
    }
}

#[cfg(all(feature = "axum", feature = "webrtc-transport"))]
pub use axum_handlers::*;

#[cfg(all(feature = "axum", not(feature = "webrtc-transport")))]
compile_error!("feature `axum` requires `webrtc-transport`");
