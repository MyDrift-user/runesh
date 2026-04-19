//! Axum WebSocket handlers for remote desktop sharing with multi-cursor support.
//!
//! # Usage
//!
//! ```ignore
//! use axum::{Router, routing::get};
//! use runesh_desktop::handlers::{ws_desktop_handler, DesktopState};
//!
//! let state = DesktopState::new(Default::default());
//! let app = Router::new()
//!     .route("/ws/desktop", get(ws_desktop_handler))
//!     .with_state(state);
//! ```

#[cfg(feature = "axum")]
mod axum_handlers {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt};

    use crate::input;
    use crate::protocol::*;
    use crate::session::{DesktopConfig, DesktopSessionManager};

    /// Shared state for desktop WebSocket handlers.
    #[derive(Clone)]
    pub struct DesktopState {
        pub session_manager: Arc<DesktopSessionManager>,
    }

    impl DesktopState {
        pub fn new(config: DesktopConfig) -> Self {
            Self {
                session_manager: Arc::new(DesktopSessionManager::new(config)),
            }
        }
    }

    /// WebSocket upgrade handler for desktop sharing.
    pub async fn ws_desktop_handler(
        ws: WebSocketUpgrade,
        State(state): State<DesktopState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| handle_desktop_ws(socket, state))
    }

    /// Main WebSocket loop for desktop sharing.
    async fn handle_desktop_ws(socket: WebSocket, state: DesktopState) {
        let (mut ws_tx, mut ws_rx) = socket.split();

        // Assign a unique cursor ID for this connection
        let cursor_id = uuid::Uuid::new_v4().to_string();
        let cursor_label = "Remote".to_string();

        // Register this connection's cursor
        if state.session_manager.multi_cursor_enabled() {
            let color = state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .add_cursor(&cursor_id, &cursor_label, false);
            tracing::info!(cursor_id = %cursor_id, color = %color, "Multi-cursor: remote cursor registered");
        }

        // Optional input injector (created on first input event)
        let mut injector: Option<Box<dyn input::InputInjector>> = None;

        // Active session frame receiver
        let mut frame_rx: Option<tokio::sync::broadcast::Receiver<crate::session::FrameUpdate>> =
            None;
        let mut active_session_id: Option<String> = None;

        // Cursor broadcast interval (60fps for smooth remote cursor movement)
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

                // Broadcast cursor positions at high frequency
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

                // Handle client messages
                msg = ws_rx.next() => {
                    let msg = match msg {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => continue,
                    };

                    let request: DesktopRequest = match serde_json::from_str(&msg) {
                        Ok(req) => req,
                        Err(e) => {
                            let err = DesktopResponse::Error {
                                code: "parse_error".into(),
                                message: format!("Invalid request: {e}"),
                            };
                            let json = serde_json::to_string(&err).unwrap_or_default();
                            let _ = ws_tx.send(Message::Text(json.into())).await;
                            continue;
                        }
                    };

                    let response = process_request(
                        &state,
                        request,
                        &cursor_id,
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

        // Cleanup: remove cursor and stop session
        if state.session_manager.multi_cursor_enabled() {
            state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .remove_cursor(&cursor_id);
            tracing::info!(cursor_id = %cursor_id, "Multi-cursor: remote cursor removed");
        }

        if let Some(session_id) = active_session_id {
            let _ = state.session_manager.stop_session(&session_id).await;
        }
    }

    async fn process_request(
        state: &DesktopState,
        request: DesktopRequest,
        cursor_id: &str,
        injector: &mut Option<Box<dyn input::InputInjector>>,
        frame_rx: &mut Option<tokio::sync::broadcast::Receiver<crate::session::FrameUpdate>>,
        active_session_id: &mut Option<String>,
    ) -> DesktopResponse {
        match request {
            DesktopRequest::StartSession {
                display_id,
                quality,
                max_fps,
            } => {
                match state
                    .session_manager
                    .start_session(Some(display_id.unwrap_or(0)), Some(quality), Some(max_fps))
                    .await
                {
                    Ok((session_id, display, rx)) => {
                        *frame_rx = Some(rx);
                        *active_session_id = Some(session_id.clone());
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
                match state.session_manager.stop_session(&session_id).await {
                    Ok(()) => {
                        *frame_rx = None;
                        *active_session_id = None;
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

            // ── Single-cursor input (backward-compatible) ─────────────
            DesktopRequest::MouseMove { x, y, .. } => {
                handle_mouse_move(state, cursor_id, x, y, injector).await
            }

            DesktopRequest::MouseButton {
                button,
                pressed,
                x,
                y,
            } => handle_mouse_button(state, cursor_id, button, pressed, x, y, injector).await,

            // ── Multi-cursor input ────────────────────────────────────
            DesktopRequest::MouseMoveCursor {
                cursor_id: cid,
                x,
                y,
                ..
            } => {
                // Use the cursor_id from the message (should match connection's cursor_id)
                handle_mouse_move(state, &cid, x, y, injector).await
            }

            DesktopRequest::MouseButtonCursor {
                cursor_id: cid,
                button,
                pressed,
                x,
                y,
            } => handle_mouse_button(state, &cid, button, pressed, x, y, injector).await,

            DesktopRequest::SetCursorMode { mode } => {
                if state.session_manager.multi_cursor_enabled() {
                    state.session_manager.cursor_tracker().write().await.mode = mode;
                    tracing::info!(mode = ?mode, "Multi-cursor mode changed");
                }
                DesktopResponse::SessionStopped {
                    session_id: "cursor_mode_updated".into(),
                }
            }

            // ── Other events ──────────────────────────────────────────
            DesktopRequest::KeyEvent {
                key_code,
                pressed,
                modifiers,
            } => {
                if state.session_manager.allow_input() {
                    // Key events go through if this cursor has focus
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
                        if let Some(inj) = injector {
                            let _ = inj.key_event(key_code, pressed, modifiers);
                        }
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
                if state.session_manager.allow_input() {
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
                        if let Some(inj) = injector {
                            let _ = inj.scroll(x, y, delta_x, delta_y);
                        }
                    }
                }
                DesktopResponse::CursorUpdate {
                    x,
                    y,
                    visible: true,
                }
            }

            DesktopRequest::SetClipboard { content } => {
                #[cfg(feature = "clipboard")]
                {
                    if state.session_manager.allow_clipboard() {
                        if let Ok(mut cb) = crate::clipboard::ClipboardManager::new() {
                            let _ = cb.set_text(&content);
                        }
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
        cursor_id: &str,
        x: i32,
        y: i32,
        injector: &mut Option<Box<dyn input::InputInjector>>,
    ) -> DesktopResponse {
        // Always update the cursor tracker position
        if state.session_manager.multi_cursor_enabled() {
            state
                .session_manager
                .cursor_tracker()
                .write()
                .await
                .update_position(cursor_id, x, y);
        }

        // Only inject OS-level input if this cursor has focus
        if state.session_manager.allow_input() {
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
    async fn handle_mouse_button(
        state: &DesktopState,
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

            // In Collaborative mode, clicking transfers focus
            if tracker.mode == MultiCursorMode::Collaborative && pressed {
                tracker.set_focus(cursor_id);
            }
        }

        if state.session_manager.allow_input() {
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
        if injector.is_none() {
            if let Ok(inj) = input::create_injector() {
                *injector = Some(inj);
            }
        }
    }
}

#[cfg(feature = "axum")]
pub use axum_handlers::*;
