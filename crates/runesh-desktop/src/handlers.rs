//! Axum WebSocket handlers for remote desktop sharing with multi-cursor support.
//!
//! # Usage
//!
//! ```ignore
//! use std::sync::Arc;
//! use axum::{Router, routing::get};
//! use runesh_desktop::handlers::{ws_desktop_handler, DesktopState};
//! use runesh_desktop::auth::{DenyAllAuth, AlwaysDeny};
//!
//! let state = DesktopState::new(
//!     Default::default(),
//!     Arc::new(DenyAllAuth),
//!     Arc::new(AlwaysDeny),
//! );
//! let app = Router::new()
//!     .route("/ws/desktop", get(ws_desktop_handler))
//!     .with_state(state);
//! ```

#[cfg(feature = "axum")]
mod axum_handlers {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::State;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt};

    use crate::auth::{ConsentBroker, DesktopAuth, Operation, Principal};
    use crate::input;
    use crate::protocol::*;
    use crate::session::{DesktopConfig, DesktopSessionManager};

    /// WebSocket frame caps.
    const MAX_FRAME_SIZE: usize = 1 << 20; // 1 MiB
    const MAX_MESSAGE_SIZE: usize = 4 << 20; // 4 MiB
    const AUTH_DEADLINE: Duration = Duration::from_secs(5);

    /// Shared state for desktop WebSocket handlers.
    #[derive(Clone)]
    pub struct DesktopState {
        pub session_manager: Arc<DesktopSessionManager>,
        pub auth: Arc<dyn DesktopAuth>,
        pub consent: Arc<dyn ConsentBroker>,
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
            }
        }
    }

    /// Per-connection runtime state that must never be forgeable from the wire.
    struct ConnState {
        /// Server-assigned cursor id, bound to this WebSocket.
        cursor_id: String,
        principal: Principal,
        /// Consent to inject input, decided once per session.
        input_consent: bool,
        /// Last clipboard write timestamps for rate limiting.
        clip_writes: std::collections::VecDeque<Instant>,
    }

    /// WebSocket upgrade handler for desktop sharing.
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
            serde_json::to_string(&DesktopResponse::Error {
                code: code.into(),
                message: msg.into(),
            })
            .unwrap_or_default()
            .into(),
        )
    }

    /// Main WebSocket loop for desktop sharing.
    async fn handle_desktop_ws(socket: WebSocket, state: DesktopState) {
        let (mut ws_tx, mut ws_rx) = socket.split();

        // 1. Authenticate (first frame must be {"type":"auth","token":...}).
        #[derive(serde::Deserialize)]
        struct AuthFrame {
            r#type: String,
            token: String,
        }

        let text = match tokio::time::timeout(AUTH_DEADLINE, ws_rx.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => t,
            _ => {
                let _ = ws_tx
                    .send(err_frame(
                        "unauthenticated",
                        "auth frame required within 5s",
                    ))
                    .await;
                return;
            }
        };
        let frame: AuthFrame = match serde_json::from_str::<AuthFrame>(&text) {
            Ok(f) if f.r#type == "auth" => f,
            _ => {
                let _ = ws_tx
                    .send(err_frame(
                        "unauthenticated",
                        "first frame must be {\"type\":\"auth\",\"token\":...}",
                    ))
                    .await;
                return;
            }
        };
        let principal = match state.auth.authenticate(&frame.token).await {
            Ok(p) => p,
            Err(e) => {
                let _ = ws_tx
                    .send(err_frame("unauthenticated", &e.to_string()))
                    .await;
                return;
            }
        };

        // 2. Server assigns the cursor id. Never trust a client-supplied cid.
        let mut conn = ConnState {
            cursor_id: uuid::Uuid::new_v4().to_string(),
            principal,
            input_consent: false,
            clip_writes: std::collections::VecDeque::new(),
        };
        let cursor_label = conn
            .principal
            .display_name
            .clone()
            .unwrap_or_else(|| "Remote".to_string());

        if state.session_manager.multi_cursor_enabled() {
            let color = state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .add_cursor(&conn.cursor_id, &cursor_label, false);
            tracing::info!(
                cursor_id = %conn.cursor_id,
                subject = %conn.principal.subject,
                color = %color,
                "desktop ws: authenticated and cursor registered"
            );
        }

        let mut injector: Option<Box<dyn input::InputInjector>> = None;
        let mut frame_rx: Option<tokio::sync::broadcast::Receiver<crate::session::FrameUpdate>> =
            None;
        let mut active_session_id: Option<String> = None;

        let mut cursor_interval = tokio::time::interval(tokio::time::Duration::from_millis(16));
        cursor_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Forward captured frames to the client
                frame = async {
                    if let Some(rx) = &mut frame_rx {
                        rx.recv().await.ok()
                    } else {
                        std::future::pending::<Option<crate::session::FrameUpdate>>().await
                    }
                } => {
                    if let Some(frame) = frame {
                        let response = DesktopResponse::Frame {
                            session_id: active_session_id.clone().unwrap_or_default(),
                            display_id: 0,
                            data: base64::engine::general_purpose::STANDARD.encode(&frame.data),
                            encoding: frame.encoding,
                            width: frame.width,
                            height: frame.height,
                            timestamp: frame.timestamp,
                            is_key_frame: frame.is_key_frame,
                        };

                        let json = serde_json::to_string(&response).unwrap_or_default();
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                }

                _ = cursor_interval.tick() => {
                    if state.session_manager.multi_cursor_enabled() && active_session_id.is_some() {
                        let cursors = state
                            .session_manager
                            .cursor_tracker()
                            .read()
                            .await
                            .all_cursors();

                        if !cursors.is_empty() {
                            let response = DesktopResponse::CursorPositions { cursors };
                            let json = serde_json::to_string(&response).unwrap_or_default();
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }

                msg = ws_rx.next() => {
                    let msg = match msg {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => continue,
                    };

                    let request: DesktopRequest = match serde_json::from_str(&msg) {
                        Ok(req) => req,
                        Err(e) => {
                            let _ = ws_tx.send(err_frame(
                                "parse_error",
                                &format!("Invalid request: {e}"),
                            )).await;
                            continue;
                        }
                    };

                    let response = process_request(
                        &state,
                        &mut conn,
                        request,
                        &mut injector,
                        &mut frame_rx,
                        &mut active_session_id,
                    ).await;

                    let json = serde_json::to_string(&response).unwrap_or_default();
                    if ws_tx.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        }

        // Cleanup
        if state.session_manager.multi_cursor_enabled() {
            state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .remove_cursor(&conn.cursor_id);
            tracing::info!(cursor_id = %conn.cursor_id, "desktop ws: cursor removed");
        }

        if let Some(session_id) = active_session_id {
            let _ = state.session_manager.stop_session(&session_id).await;
        }
    }

    fn authz(
        state: &DesktopState,
        principal: &Principal,
        op: &Operation,
    ) -> Result<(), DesktopResponse> {
        state
            .auth
            .authorize(principal, op)
            .map_err(|e| DesktopResponse::Error {
                code: "forbidden".into(),
                message: e.to_string(),
            })
    }

    async fn process_request(
        state: &DesktopState,
        conn: &mut ConnState,
        request: DesktopRequest,
        injector: &mut Option<Box<dyn input::InputInjector>>,
        frame_rx: &mut Option<tokio::sync::broadcast::Receiver<crate::session::FrameUpdate>>,
        active_session_id: &mut Option<String>,
    ) -> DesktopResponse {
        // Server-owned cursor id; never trust the client value.
        let cursor_id = conn.cursor_id.clone();

        match request {
            DesktopRequest::StartSession {
                display_id,
                quality,
                max_fps,
            } => {
                if let Err(e) = authz(state, &conn.principal, &Operation::StartSession) {
                    return e;
                }
                match state
                    .session_manager
                    .start_session(Some(display_id.unwrap_or(0)), Some(quality), Some(max_fps))
                    .await
                {
                    Ok((session_id, display, rx)) => {
                        *frame_rx = Some(rx);
                        *active_session_id = Some(session_id.clone());
                        // Ask for input consent once per session. Default deny.
                        conn.input_consent = state
                            .consent
                            .request_input_consent(&session_id, &conn.principal)
                            .await;
                        tracing::info!(
                            session_id = %session_id,
                            subject = %conn.principal.subject,
                            input_consent = conn.input_consent,
                            "desktop session started"
                        );
                        DesktopResponse::SessionStarted {
                            session_id,
                            display,
                        }
                    }
                    Err(e) => DesktopResponse::Error {
                        code: e.error_code().into(),
                        message: e.to_string(),
                    },
                }
            }

            DesktopRequest::StopSession { session_id } => {
                if let Err(e) = authz(state, &conn.principal, &Operation::StopSession) {
                    return e;
                }
                match state.session_manager.stop_session(&session_id).await {
                    Ok(()) => {
                        *frame_rx = None;
                        *active_session_id = None;
                        conn.input_consent = false;
                        DesktopResponse::SessionStopped { session_id }
                    }
                    Err(e) => DesktopResponse::Error {
                        code: e.error_code().into(),
                        message: e.to_string(),
                    },
                }
            }

            DesktopRequest::ListDisplays => match crate::display::enumerate_displays() {
                Ok(displays) => DesktopResponse::Displays { displays },
                Err(e) => DesktopResponse::Error {
                    code: e.error_code().into(),
                    message: e.to_string(),
                },
            },

            DesktopRequest::SetQuality { quality } => {
                if let Some(session_id) = active_session_id {
                    let _ = state.session_manager.set_quality(session_id, quality).await;
                }
                DesktopResponse::SessionStopped {
                    session_id: "quality_updated".into(),
                }
            }

            DesktopRequest::MouseMove { x, y, .. } => {
                handle_mouse_move(state, conn, &cursor_id, x, y, injector).await
            }

            DesktopRequest::MouseButton {
                button,
                pressed,
                x,
                y,
            } => {
                handle_mouse_button(state, conn, &cursor_id, button, pressed, x, y, injector).await
            }

            // Cursor-id fields from the client are ignored. Server uses the
            // connection's cursor id, so one client cannot steal focus or
            // move another cursor.
            DesktopRequest::MouseMoveCursor { x, y, .. } => {
                handle_mouse_move(state, conn, &cursor_id, x, y, injector).await
            }

            DesktopRequest::MouseButtonCursor {
                button,
                pressed,
                x,
                y,
                ..
            } => {
                handle_mouse_button(state, conn, &cursor_id, button, pressed, x, y, injector).await
            }

            DesktopRequest::SetCursorMode { mode } => {
                if state.session_manager.multi_cursor_enabled() {
                    state.session_manager.cursor_tracker().write().await.mode = mode;
                    tracing::info!(mode = ?mode, "multi-cursor mode changed");
                }
                DesktopResponse::SessionStopped {
                    session_id: "cursor_mode_updated".into(),
                }
            }

            DesktopRequest::KeyEvent {
                key_code,
                pressed,
                modifiers,
            } => {
                if state.session_manager.allow_input() && conn.input_consent {
                    if let Err(e) = authz(state, &conn.principal, &Operation::InjectInput) {
                        return e;
                    }
                    let should_inject = if state.session_manager.multi_cursor_enabled() {
                        state
                            .session_manager
                            .cursor_tracker()
                            .read()
                            .await
                            .should_inject_input(&cursor_id)
                    } else {
                        true
                    };

                    if should_inject && let Some(inj) = injector {
                        let _ = inj.key_event(key_code, pressed, modifiers);
                    }
                }
                DesktopResponse::SessionStopped {
                    session_id: "key_processed".into(),
                }
            }

            DesktopRequest::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                if state.session_manager.allow_input() && conn.input_consent {
                    if let Err(e) = authz(state, &conn.principal, &Operation::InjectInput) {
                        return e;
                    }
                    let should_inject = if state.session_manager.multi_cursor_enabled() {
                        state
                            .session_manager
                            .cursor_tracker()
                            .read()
                            .await
                            .should_inject_input(&cursor_id)
                    } else {
                        true
                    };

                    if should_inject && let Some(inj) = injector {
                        let _ = inj.scroll(x, y, delta_x, delta_y);
                    }
                }
                DesktopResponse::CursorUpdate {
                    x,
                    y,
                    visible: true,
                }
            }

            DesktopRequest::SetClipboard { content } => {
                let clip = state.session_manager.clipboard_settings();
                if !state.session_manager.allow_clipboard()
                    || !clip.direction.allows_viewer_to_host()
                {
                    return DesktopResponse::Error {
                        code: "clipboard_disabled".into(),
                        message: "viewer-to-host clipboard not allowed".into(),
                    };
                }
                if content.len() > clip.max_bytes {
                    return DesktopResponse::Error {
                        code: "clipboard_too_large".into(),
                        message: format!("clipboard payload exceeds {} bytes", clip.max_bytes),
                    };
                }
                // Simple token-bucket style rate limit.
                let now = Instant::now();
                let window = Duration::from_secs(1);
                while let Some(&front) = conn.clip_writes.front() {
                    if now.duration_since(front) > window {
                        conn.clip_writes.pop_front();
                    } else {
                        break;
                    }
                }
                if conn.clip_writes.len() as u32 >= clip.write_rate_per_sec {
                    return DesktopResponse::Error {
                        code: "clipboard_rate_limited".into(),
                        message: "too many clipboard writes".into(),
                    };
                }
                conn.clip_writes.push_back(now);
                if let Err(e) = authz(state, &conn.principal, &Operation::SetClipboard) {
                    return e;
                }
                #[cfg(feature = "clipboard")]
                {
                    if let Ok(mut cb) = crate::clipboard::ClipboardManager::new() {
                        let _ = cb.set_text(&content);
                    }
                }
                DesktopResponse::ClipboardUpdate { content }
            }

            DesktopRequest::SelectDisplay { display_id } => DesktopResponse::SessionStopped {
                session_id: format!("display_{display_id}_selected"),
            },

            DesktopRequest::RequestKeyFrame => DesktopResponse::SessionStopped {
                session_id: "keyframe_requested".into(),
            },
        }
    }

    /// Handle mouse move with multi-cursor awareness.
    async fn handle_mouse_move(
        state: &DesktopState,
        conn: &ConnState,
        cursor_id: &str,
        x: i32,
        y: i32,
        injector: &mut Option<Box<dyn input::InputInjector>>,
    ) -> DesktopResponse {
        if state.session_manager.multi_cursor_enabled() {
            state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .update_position(cursor_id, x, y);
        }

        if state.session_manager.allow_input() && conn.input_consent {
            if let Err(e) = authz(state, &conn.principal, &Operation::InjectInput) {
                return e;
            }
            let should_inject = if state.session_manager.multi_cursor_enabled() {
                state
                    .session_manager
                    .cursor_tracker()
                    .read()
                    .await
                    .should_inject_input(cursor_id)
            } else {
                true
            };

            if should_inject {
                ensure_injector(injector);
                if let Some(inj) = injector {
                    let _ = inj.mouse_move(x, y);
                }
            }
        }

        DesktopResponse::CursorUpdate {
            x,
            y,
            visible: true,
        }
    }

    /// Handle mouse button with multi-cursor focus transfer.
    #[allow(clippy::too_many_arguments)]
    async fn handle_mouse_button(
        state: &DesktopState,
        conn: &ConnState,
        cursor_id: &str,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
        injector: &mut Option<Box<dyn input::InputInjector>>,
    ) -> DesktopResponse {
        if state.session_manager.multi_cursor_enabled() {
            let mut tracker = state.session_manager.cursor_tracker().write().await;
            tracker.update_position(cursor_id, x, y);
            if tracker.mode == MultiCursorMode::Collaborative && pressed {
                tracker.set_focus(cursor_id);
            }
        }

        if state.session_manager.allow_input() && conn.input_consent {
            if let Err(e) = authz(state, &conn.principal, &Operation::InjectInput) {
                return e;
            }
            let should_inject = if state.session_manager.multi_cursor_enabled() {
                state
                    .session_manager
                    .cursor_tracker()
                    .read()
                    .await
                    .should_inject_input(cursor_id)
            } else {
                true
            };

            if should_inject {
                ensure_injector(injector);
                if let Some(inj) = injector {
                    let _ = inj.mouse_button(button, pressed, x, y);
                }
            }
        }

        DesktopResponse::CursorUpdate {
            x,
            y,
            visible: true,
        }
    }

    /// Lazily create the input injector.
    fn ensure_injector(injector: &mut Option<Box<dyn input::InputInjector>>) {
        if injector.is_none()
            && let Ok(inj) = input::create_injector()
        {
            *injector = Some(inj);
        }
    }
}

#[cfg(feature = "axum")]
pub use axum_handlers::*;
