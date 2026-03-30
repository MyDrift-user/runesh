//! Desktop sharing session management.
//!
//! Manages active screen sharing sessions with capture loops,
//! quality adaptation, and resource cleanup.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, RwLock};

use crate::capture;
use crate::cursor::CursorTracker;
use crate::encode;
use crate::error::DesktopError;
use crate::protocol::{DisplayInfo, Encoding, MultiCursorMode, Quality};

/// Configuration for desktop sharing sessions.
#[derive(Debug, Clone)]
pub struct DesktopConfig {
    /// Maximum concurrent sessions.
    pub max_sessions: usize,
    /// Default quality setting.
    pub default_quality: Quality,
    /// Default max FPS.
    pub default_max_fps: u32,
    /// Session idle timeout.
    pub idle_timeout_secs: u64,
    /// Whether to allow remote input injection.
    pub allow_input: bool,
    /// Whether to enable clipboard sharing.
    pub allow_clipboard: bool,
    /// Default multi-cursor mode.
    pub multi_cursor_mode: MultiCursorMode,
    /// Whether to enable multi-cursor support.
    pub enable_multi_cursor: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            max_sessions: 5,
            default_quality: Quality::Medium,
            default_max_fps: 30,
            idle_timeout_secs: 3600,
            allow_input: true,
            allow_clipboard: true,
            multi_cursor_mode: MultiCursorMode::Collaborative,
            enable_multi_cursor: true,
        }
    }
}

/// A frame from the capture loop, ready for WebSocket transport.
#[derive(Clone)]
pub struct FrameUpdate {
    pub data: Vec<u8>,
    pub encoding: Encoding,
    pub width: u32,
    pub height: u32,
    pub is_key_frame: bool,
    pub timestamp: u64,
}

/// Active session state.
struct SessionState {
    _display_id: u32,
    quality: Quality,
    _max_fps: u32,
    _frame_tx: broadcast::Sender<FrameUpdate>,
    cancel_tx: tokio::sync::oneshot::Sender<()>,
    _created_at: Instant,
}

/// Manages desktop sharing sessions.
pub struct DesktopSessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    config: DesktopConfig,
    /// Shared cursor tracker for multi-cursor support.
    cursor_tracker: Arc<RwLock<CursorTracker>>,
}

impl DesktopSessionManager {
    pub fn new(config: DesktopConfig) -> Self {
        let mode = config.multi_cursor_mode;
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cursor_tracker: Arc::new(RwLock::new(CursorTracker::new(mode))),
            config,
        }
    }

    /// Get a reference to the shared cursor tracker.
    pub fn cursor_tracker(&self) -> &Arc<RwLock<CursorTracker>> {
        &self.cursor_tracker
    }

    /// Check if multi-cursor is enabled.
    pub fn multi_cursor_enabled(&self) -> bool {
        self.config.enable_multi_cursor
    }

    /// Start a new desktop sharing session.
    /// Returns (session_id, display_info, frame_receiver).
    pub async fn start_session(
        &self,
        display_id: Option<u32>,
        quality: Option<Quality>,
        max_fps: Option<u32>,
    ) -> Result<(String, DisplayInfo, broadcast::Receiver<FrameUpdate>), DesktopError> {
        let sessions = self.sessions.read().await;
        if sessions.len() >= self.config.max_sessions {
            return Err(DesktopError::MaxSessions);
        }
        drop(sessions);

        let display_id = display_id.unwrap_or(0);
        let quality = quality.unwrap_or(self.config.default_quality);
        let max_fps = max_fps.unwrap_or(self.config.default_max_fps);

        // Get display info
        let displays = crate::display::enumerate_displays()?;
        let display = displays
            .into_iter()
            .find(|d| d.id == display_id)
            .ok_or(DesktopError::DisplayNotFound(display_id))?;

        let session_id = uuid::Uuid::new_v4().to_string();
        let (frame_tx, frame_rx) = broadcast::channel(4);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

        // Start the capture loop in a blocking thread
        let frame_tx_clone = frame_tx.clone();
        let capture_display_id = display_id;

        tokio::task::spawn_blocking(move || {
            capture_loop(capture_display_id, quality, max_fps, frame_tx_clone, cancel_rx)
        });

        self.sessions.write().await.insert(
            session_id.clone(),
            SessionState {
                _display_id: display_id,
                quality,
                _max_fps: max_fps,
                _frame_tx: frame_tx,
                cancel_tx,
                _created_at: Instant::now(),
            },
        );

        tracing::info!(
            session_id = %session_id,
            display_id,
            quality = ?quality,
            max_fps,
            "Desktop sharing session started"
        );

        Ok((session_id, display, frame_rx))
    }

    /// Stop a desktop sharing session.
    pub async fn stop_session(&self, session_id: &str) -> Result<(), DesktopError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .remove(session_id)
            .ok_or_else(|| DesktopError::SessionNotFound(session_id.into()))?;

        // Signal the capture loop to stop
        let _ = session.cancel_tx.send(());

        tracing::info!(session_id = %session_id, "Desktop sharing session stopped");
        Ok(())
    }

    /// Update quality for an active session.
    pub async fn set_quality(
        &self,
        session_id: &str,
        quality: Quality,
    ) -> Result<(), DesktopError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| DesktopError::SessionNotFound(session_id.into()))?;
        session.quality = quality;
        Ok(())
    }

    /// Check if input injection is allowed.
    pub fn allow_input(&self) -> bool {
        self.config.allow_input
    }

    /// Check if clipboard sharing is allowed.
    pub fn allow_clipboard(&self) -> bool {
        self.config.allow_clipboard
    }
}

/// Main capture loop: runs in a blocking thread, captures and encodes frames.
fn capture_loop(
    display_id: u32,
    quality: Quality,
    max_fps: u32,
    frame_tx: broadcast::Sender<FrameUpdate>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut capturer = match capture::create_capturer(display_id) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create screen capturer");
            return;
        }
    };

    let frame_interval = Duration::from_millis(1000 / max_fps.max(1) as u64);
    let encoding = encode::auto_encoding(quality);

    loop {
        // Check for cancellation
        if cancel_rx.try_recv().is_ok() {
            tracing::debug!("Capture loop cancelled");
            break;
        }

        // If no receivers, stop
        if frame_tx.receiver_count() == 0 {
            tracing::debug!("No receivers, stopping capture loop");
            break;
        }

        let frame_start = Instant::now();

        match capturer.capture_frame() {
            Ok(frame) => {
                match encode::encode_frame(&frame, quality, encoding) {
                    Ok(encoded) => {
                        let update = FrameUpdate {
                            data: encoded.data,
                            encoding: encoded.encoding,
                            width: encoded.width,
                            height: encoded.height,
                            is_key_frame: encoded.is_key_frame,
                            timestamp: frame.timestamp,
                        };
                        let _ = frame_tx.send(update);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Frame encoding failed");
                    }
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "Frame capture failed (may be transient)");
            }
        }

        // Frame rate limiting
        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            std::thread::sleep(frame_interval - elapsed);
        }
    }
}
