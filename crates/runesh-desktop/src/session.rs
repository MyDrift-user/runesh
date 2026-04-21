//! Desktop sharing sessions.
//!
//! One **video pipeline** runs per `(display_id)` and fans out encoded H.264
//! samples via a [`broadcast`] channel. N peers can subscribe simultaneously
//! and each peer writes the samples to its own RTP track.
//!
//! Similarly, one **audio pipeline** runs per session owner and fans out
//! encoded Opus packets.
//!
//! The pipelines stop automatically when the last subscriber disconnects.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, broadcast};

use crate::clipboard::ClipboardSettings;
use crate::cursor::CursorTracker;
use crate::encode::VideoSample;
#[cfg(feature = "audio")]
use crate::encode::opus_enc::AudioSample;
use crate::error::DesktopError;
use crate::protocol::{DisplayInfo, MultiCursorMode, Quality};

// ── Configuration ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DesktopConfig {
    pub max_sessions: usize,
    pub default_quality: Quality,
    pub default_max_fps: u32,
    pub idle_timeout_secs: u64,
    pub allow_input: bool,
    pub allow_clipboard: bool,
    pub clipboard: ClipboardSettings,
    pub multi_cursor_mode: MultiCursorMode,
    pub enable_multi_cursor: bool,
    /// When true the audio pipeline is launched per-session.
    pub enable_audio: bool,
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
            clipboard: ClipboardSettings::default(),
            multi_cursor_mode: MultiCursorMode::Collaborative,
            enable_multi_cursor: true,
            enable_audio: cfg!(feature = "audio"),
        }
    }
}

// ── Video fan-out pipeline ────────────────────────────────────────────────

/// A single live capture + encode loop for one display.
pub struct VideoPipeline {
    sample_tx: broadcast::Sender<Arc<VideoSample>>,
    /// Kept alive so that dropping the pipeline tears down the capture thread.
    #[allow(dead_code)]
    cancel_tx: tokio::sync::oneshot::Sender<()>,
    force_keyframe: Arc<std::sync::atomic::AtomicBool>,
    display: DisplayInfo,
    #[allow(dead_code)]
    created_at: Instant,
}

impl VideoPipeline {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<VideoSample>> {
        self.sample_tx.subscribe()
    }

    pub fn subscribers(&self) -> usize {
        self.sample_tx.receiver_count()
    }

    pub fn request_keyframe(&self) {
        self.force_keyframe
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn display(&self) -> &DisplayInfo {
        &self.display
    }
}

/// Spawn a capture + encode loop for the given display. The returned
/// [`VideoPipeline`] fans out encoded samples to any number of subscribers.
pub fn spawn_video_pipeline(
    display: DisplayInfo,
    quality: Quality,
    max_fps: u32,
    cursor_tracker: Arc<RwLock<CursorTracker>>,
) -> Result<VideoPipeline, DesktopError> {
    let (sample_tx, _) = broadcast::channel::<Arc<VideoSample>>(8);
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let force_keyframe = Arc::new(std::sync::atomic::AtomicBool::new(true));

    let display_id = display.id;
    let sample_tx_clone = sample_tx.clone();
    let force_kf_clone = force_keyframe.clone();

    // Capture + encode is blocking work (DXGI, CG, X11 all block). Run it
    // on a dedicated OS thread so it never starves the tokio runtime.
    std::thread::Builder::new()
        .name(format!("runesh-desktop-video-{display_id}"))
        .spawn(move || {
            run_video_pipeline(
                display_id,
                quality,
                max_fps,
                sample_tx_clone,
                force_kf_clone,
                cursor_tracker,
                &mut cancel_rx,
            );
        })
        .map_err(|e| DesktopError::Internal(format!("spawn video thread: {e}")))?;

    Ok(VideoPipeline {
        sample_tx,
        cancel_tx,
        force_keyframe,
        display,
        created_at: Instant::now(),
    })
}

fn run_video_pipeline(
    display_id: u32,
    quality: Quality,
    max_fps: u32,
    sample_tx: broadcast::Sender<Arc<VideoSample>>,
    force_keyframe: Arc<std::sync::atomic::AtomicBool>,
    cursor_tracker: Arc<RwLock<CursorTracker>>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) {
    let mut capturer = match crate::capture::create_capturer(display_id) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, display_id, "failed to start capturer");
            return;
        }
    };
    let (width, height) = capturer.dimensions();

    let mut encoder = match crate::encode::create_video_encoder(width, height, quality, max_fps) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, width, height, "failed to start video encoder");
            return;
        }
    };

    let frame_interval = Duration::from_millis(1000 / max_fps.max(1) as u64);
    let mut next_deadline = Instant::now();

    loop {
        if cancel_rx.try_recv().is_ok() {
            break;
        }
        if sample_tx.receiver_count() == 0 {
            // Idle: no viewers. Sleep briefly and check again.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Honour pending keyframe requests (e.g. new peer just joined).
        if force_keyframe.swap(false, std::sync::atomic::Ordering::Relaxed) {
            encoder.force_keyframe();
        }

        let mut frame = match capturer.capture_frame() {
            Ok(f) => f,
            Err(e) => {
                tracing::trace!(error = %e, "transient capture error");
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        };

        // Composite overlay cursors before encoding.
        if let Ok(tracker) = cursor_tracker.try_read() {
            crate::cursor::composite_cursors(&mut frame, &tracker);
        }

        match encoder.encode(&frame) {
            Ok(Some(sample)) => {
                let _ = sample_tx.send(Arc::new(sample));
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "video encode failed");
            }
        }

        // Frame pacer: absolute deadline, not sleep-from-now, to avoid drift.
        next_deadline += frame_interval;
        let now = Instant::now();
        if next_deadline > now {
            std::thread::sleep(next_deadline - now);
        } else {
            // Drifted more than one frame behind — catch up and stop sleeping.
            next_deadline = now;
        }
    }

    tracing::info!(display_id, "video pipeline exited");
}

// ── Audio fan-out pipeline ────────────────────────────────────────────────

#[cfg(feature = "audio")]
pub struct AudioPipeline {
    sample_tx: broadcast::Sender<Arc<AudioSample>>,
    #[allow(dead_code)]
    cancel_tx: tokio::sync::oneshot::Sender<()>,
}

#[cfg(feature = "audio")]
impl AudioPipeline {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AudioSample>> {
        self.sample_tx.subscribe()
    }
    pub fn subscribers(&self) -> usize {
        self.sample_tx.receiver_count()
    }
}

#[cfg(feature = "audio")]
pub fn spawn_audio_pipeline() -> Result<AudioPipeline, DesktopError> {
    use crate::capture::audio::AudioCapturer;
    use crate::encode::opus_enc::OpusSampleEncoder;

    let (sample_tx, _) = broadcast::channel::<Arc<AudioSample>>(32);
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let sample_tx_clone = sample_tx.clone();

    std::thread::Builder::new()
        .name("runesh-desktop-audio".into())
        .spawn(move || {
            let capturer = match AudioCapturer::start() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "audio capture start failed");
                    return;
                }
            };
            let channels = capturer.channels().clamp(1, 2);
            let mut encoder = match OpusSampleEncoder::new(channels, 96) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!(error = %e, "opus encoder start failed");
                    return;
                }
            };
            tracing::info!(channels, "audio pipeline running");
            loop {
                if cancel_rx.try_recv().is_ok() {
                    break;
                }
                if sample_tx_clone.receiver_count() == 0 {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                let Some(frame) = capturer.next_frame() else {
                    break;
                };
                match encoder.encode_frame(&frame.samples) {
                    Ok(sample) => {
                        let _ = sample_tx_clone.send(Arc::new(sample));
                    }
                    Err(e) => tracing::warn!(error = %e, "opus encode failed"),
                }
            }
            tracing::info!("audio pipeline exited");
        })
        .map_err(|e| DesktopError::Internal(format!("spawn audio thread: {e}")))?;

    Ok(AudioPipeline {
        sample_tx,
        cancel_tx,
    })
}

// ── Session manager ───────────────────────────────────────────────────────

#[allow(dead_code)]
struct SessionState {
    session_id: String,
    display_id: u32,
    quality: Quality,
    max_fps: u32,
    created_at: Instant,
}

/// Top-level manager. Holds video and audio pipelines and maps logical
/// session ids to the underlying capture pipeline they use.
pub struct DesktopSessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    /// Video pipelines are keyed by `display_id` and shared across sessions
    /// that want the same display.
    video_pipelines: Arc<RwLock<HashMap<u32, Arc<VideoPipeline>>>>,
    #[cfg(feature = "audio")]
    audio_pipeline: Arc<RwLock<Option<Arc<AudioPipeline>>>>,
    cursor_tracker: Arc<RwLock<CursorTracker>>,
    config: DesktopConfig,
}

impl DesktopSessionManager {
    pub fn new(config: DesktopConfig) -> Self {
        let cursor_tracker = Arc::new(RwLock::new(CursorTracker::new(config.multi_cursor_mode)));
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            video_pipelines: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "audio")]
            audio_pipeline: Arc::new(RwLock::new(None)),
            cursor_tracker,
            config,
        }
    }

    pub fn cursor_tracker(&self) -> &Arc<RwLock<CursorTracker>> {
        &self.cursor_tracker
    }

    pub fn multi_cursor_enabled(&self) -> bool {
        self.config.enable_multi_cursor
    }
    pub fn allow_input(&self) -> bool {
        self.config.allow_input
    }
    pub fn allow_clipboard(&self) -> bool {
        self.config.allow_clipboard
    }
    pub fn clipboard_settings(&self) -> &ClipboardSettings {
        &self.config.clipboard
    }
    pub fn audio_enabled(&self) -> bool {
        self.config.enable_audio
    }

    /// Start (or join) a session on the given display. Returns the session id,
    /// the resolved display info, and a shared pointer to the live video pipeline.
    pub async fn start_session(
        &self,
        display_id: Option<u32>,
        quality: Option<Quality>,
        max_fps: Option<u32>,
    ) -> Result<(String, DisplayInfo, Arc<VideoPipeline>), DesktopError> {
        let sessions = self.sessions.read().await;
        if sessions.len() >= self.config.max_sessions {
            return Err(DesktopError::MaxSessions);
        }
        drop(sessions);

        let display_id = display_id.unwrap_or(0);
        let quality = quality.unwrap_or(self.config.default_quality);
        let max_fps = max_fps.unwrap_or(self.config.default_max_fps);

        let displays = crate::display::enumerate_displays()?;
        let display = displays
            .into_iter()
            .find(|d| d.id == display_id)
            .ok_or(DesktopError::DisplayNotFound(display_id))?;

        let mut map = self.video_pipelines.write().await;
        let pipeline = if let Some(p) = map.get(&display_id) {
            Arc::clone(p)
        } else {
            let p = Arc::new(spawn_video_pipeline(
                display.clone(),
                quality,
                max_fps,
                Arc::clone(&self.cursor_tracker),
            )?);
            map.insert(display_id, Arc::clone(&p));
            p
        };
        drop(map);

        // New viewer → ask encoder to emit a keyframe so they can decode immediately.
        pipeline.request_keyframe();

        let session_id = uuid::Uuid::new_v4().to_string();
        self.sessions.write().await.insert(
            session_id.clone(),
            SessionState {
                session_id: session_id.clone(),
                display_id,
                quality,
                max_fps,
                created_at: Instant::now(),
            },
        );

        tracing::info!(
            session_id = %session_id,
            display_id,
            quality = ?quality,
            max_fps,
            "desktop session started"
        );

        Ok((session_id, display, pipeline))
    }

    /// Stop a logical session. When the last session on a given display goes
    /// away the underlying pipeline is torn down.
    pub async fn stop_session(&self, session_id: &str) -> Result<(), DesktopError> {
        let mut sessions = self.sessions.write().await;
        let removed = sessions
            .remove(session_id)
            .ok_or_else(|| DesktopError::SessionNotFound(session_id.into()))?;
        drop(sessions);

        // If no remaining sessions are on this display and no peers are subscribed
        // to the pipeline, drop it.
        let still_needed = {
            let s = self.sessions.read().await;
            s.values().any(|st| st.display_id == removed.display_id)
        };
        if !still_needed {
            let mut map = self.video_pipelines.write().await;
            if let Some(p) = map.get(&removed.display_id)
                && p.subscribers() == 0
            {
                map.remove(&removed.display_id);
            }
        }

        tracing::info!(session_id = %session_id, "desktop session stopped");
        Ok(())
    }

    /// Get or spawn the global audio pipeline. The pipeline is shared across
    /// all sessions and torn down when the last subscriber disconnects.
    #[cfg(feature = "audio")]
    pub async fn ensure_audio_pipeline(&self) -> Result<Arc<AudioPipeline>, DesktopError> {
        let mut slot = self.audio_pipeline.write().await;
        if let Some(p) = slot.as_ref() {
            return Ok(Arc::clone(p));
        }
        let p = Arc::new(spawn_audio_pipeline()?);
        *slot = Some(Arc::clone(&p));
        Ok(p)
    }

    /// Snapshot of the current `display_id → pipeline` map. Mostly useful for
    /// handlers that want to peek at pipelines without knowing the display id.
    pub async fn video_pipelines_snapshot(&self) -> HashMap<u32, Arc<VideoPipeline>> {
        self.video_pipelines.read().await.clone()
    }

    /// Tear down and rebuild the pipeline for a given display at new
    /// quality/fps settings. Used for runtime quality changes since
    /// OpenH264's Rust binding cannot mutate bitrate in place.
    pub async fn rebuild_video_pipeline(
        &self,
        display_id: u32,
        quality: Quality,
        max_fps: Option<u32>,
    ) -> Result<Arc<VideoPipeline>, DesktopError> {
        let displays = crate::display::enumerate_displays()?;
        let display = displays
            .into_iter()
            .find(|d| d.id == display_id)
            .ok_or(DesktopError::DisplayNotFound(display_id))?;
        let max_fps = max_fps.unwrap_or(self.config.default_max_fps);

        let new_pipeline = Arc::new(spawn_video_pipeline(
            display,
            quality,
            max_fps,
            Arc::clone(&self.cursor_tracker),
        )?);

        let mut map = self.video_pipelines.write().await;
        // Drop the old pipeline; its capture thread exits when the sender is dropped.
        map.insert(display_id, Arc::clone(&new_pipeline));
        Ok(new_pipeline)
    }

    /// Switch to a different display. Starts the target display's pipeline if
    /// one isn't already running, and returns a shared pointer to it.
    pub async fn switch_display(
        &self,
        display_id: u32,
        quality: Option<Quality>,
        max_fps: Option<u32>,
    ) -> Result<Arc<VideoPipeline>, DesktopError> {
        let displays = crate::display::enumerate_displays()?;
        let display = displays
            .into_iter()
            .find(|d| d.id == display_id)
            .ok_or(DesktopError::DisplayNotFound(display_id))?;
        let quality = quality.unwrap_or(self.config.default_quality);
        let max_fps = max_fps.unwrap_or(self.config.default_max_fps);

        let mut map = self.video_pipelines.write().await;
        if let Some(p) = map.get(&display_id) {
            return Ok(Arc::clone(p));
        }
        let p = Arc::new(spawn_video_pipeline(
            display,
            quality,
            max_fps,
            Arc::clone(&self.cursor_tracker),
        )?);
        map.insert(display_id, Arc::clone(&p));
        Ok(p)
    }
}
