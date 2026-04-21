//! Linux Wayland screen capture via xdg-desktop-portal + PipeWire.
//!
//! On Wayland the compositor never gives random apps access to the framebuffer.
//! The sanctioned path is:
//!
//! 1. Ask `org.freedesktop.portal.ScreenCast` (via `ashpd`) for a ScreenCast
//!    session; the user sees a system-level picker dialog.
//! 2. The portal returns a PipeWire node id + a file descriptor.
//! 3. We open a PipeWire stream on that node and receive `memfd` buffers in
//!    either `Rgbx` / `Bgrx` / `Rgba` / `Bgra` formats.
//! 4. Each buffer is copied into a [`CapturedFrame`] in BGRA layout (what the
//!    rest of the crate expects) and handed back.
//!
//! This module compiles only on Linux with the `wayland` feature on. `ashpd`
//! and `pipewire` are pulled in conditionally.

use std::os::unix::io::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;
use pipewire as pw;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::format_utils::parse_format;
use pw::spa::param::video::VideoFormat;
use pw::spa::pod::Pod;
use pw::stream::{Stream, StreamFlags, StreamListener};

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

/// Detect whether we're running under Wayland.
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
}

/// Live Wayland screen capture handle.
pub struct WaylandCapturer {
    width: u32,
    height: u32,
    /// Most recently received frame. Replaced in place by the PipeWire callback.
    latest: Arc<Mutex<Option<CapturedFrame>>>,
    /// Keep the mainloop alive. Dropping it ends the capture.
    _handle: PipewireMainloopHandle,
}

/// Owns the PipeWire mainloop thread so we can join it on drop.
struct PipewireMainloopHandle {
    quit_tx: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for PipewireMainloopHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.quit_tx.take() {
            let _ = tx.send(());
        }
        if let Some(th) = self.thread.take() {
            let _ = th.join();
        }
    }
}

impl WaylandCapturer {
    /// Request a ScreenCast session from xdg-desktop-portal and start reading
    /// frames from the returned PipeWire node.
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        // ashpd is async; we run it on its own tokio runtime so this constructor
        // stays synchronous (consistent with the other capture backends).
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DesktopError::Capture(format!("wayland runtime: {e}")))?;

        let (pipewire_fd, node_id, width, height) = rt
            .block_on(open_portal_session(display_id))
            .map_err(|e| DesktopError::Capture(format!("xdg-desktop-portal: {e}")))?;

        let latest: Arc<Mutex<Option<CapturedFrame>>> = Arc::new(Mutex::new(None));
        let latest_for_cb = Arc::clone(&latest);

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(u32, u32), String>>();
        let (quit_tx, quit_rx) = std::sync::mpsc::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("runesh-desktop-pipewire".into())
            .spawn(move || {
                if let Err(e) =
                    run_pipewire_mainloop(pipewire_fd, node_id, latest_for_cb, ready_tx, quit_rx)
                {
                    tracing::error!(error = %e, "pipewire mainloop exited with error");
                }
            })
            .map_err(|e| DesktopError::Capture(format!("spawn pipewire thread: {e}")))?;

        // Wait (up to 5 s) for the PipeWire stream to reach a usable state.
        let (w, h) = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                DesktopError::Capture("pipewire stream did not become ready".into())
            })?
            .map_err(DesktopError::Capture)?;

        Ok(Self {
            width: w.max(width),
            height: h.max(height),
            latest,
            _handle: PipewireMainloopHandle {
                quit_tx: Some(quit_tx),
                thread: Some(thread),
            },
        })
    }
}

impl ScreenCapture for WaylandCapturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        // Poll the latest frame. We intentionally clone (not drain) so the
        // next call still returns a frame if the PipeWire stream is briefly
        // idle (static desktop = no new buffers).
        let guard = self
            .latest
            .lock()
            .map_err(|_| DesktopError::Capture("wayland latest lock poisoned".into()))?;
        match guard.as_ref() {
            Some(f) => Ok(CapturedFrame {
                width: f.width,
                height: f.height,
                data: f.data.clone(),
                timestamp: f.timestamp,
            }),
            None => Err(DesktopError::Capture(
                "no pipewire frame ready yet".into(),
            )),
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Open a ScreenCast session via xdg-desktop-portal and return the PipeWire
/// remote file descriptor plus node id for the selected source.
async fn open_portal_session(
    display_id: u32,
) -> Result<(OwnedFd, u32, u32, u32), String> {
    let proxy = Screencast::new().await.map_err(|e| e.to_string())?;
    let session = proxy.create_session().await.map_err(|e| e.to_string())?;
    proxy
        .select_sources(
            &session,
            CursorMode::Embedded,
            SourceType::Monitor.into(),
            false,
            None,
            PersistMode::DoNot,
        )
        .await
        .map_err(|e| e.to_string())?;
    let streams = proxy
        .start(&session, None)
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;

    let stream = streams
        .streams()
        .get(display_id as usize)
        .or_else(|| streams.streams().first())
        .ok_or_else(|| "no ScreenCast streams returned".to_string())?;
    let node_id = stream.pipe_wire_node_id();
    let (w, h) = stream.size().unwrap_or((0, 0));

    let fd = proxy
        .open_pipe_wire_remote(&session)
        .await
        .map_err(|e| e.to_string())?;

    Ok((fd, node_id, w as u32, h as u32))
}

/// Run the PipeWire mainloop on a dedicated thread, consuming frames from
/// the portal-provided node and writing them to `latest`.
fn run_pipewire_mainloop(
    fd: OwnedFd,
    node_id: u32,
    latest: Arc<Mutex<Option<CapturedFrame>>>,
    ready_tx: std::sync::mpsc::Sender<Result<(u32, u32), String>>,
    quit_rx: std::sync::mpsc::Receiver<()>,
) -> Result<(), String> {
    pw::init();

    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::Context::new(&mainloop).map_err(|e| e.to_string())?;
    let core = context
        .connect_fd(fd.as_raw_fd(), None)
        .map_err(|e| e.to_string())?;

    let stream = Stream::new(
        &core,
        "runesh-desktop-wayland-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;

    let mut listener_width: u32 = 0;
    let mut listener_height: u32 = 0;
    let mut ready_sent = false;
    let latest_for_process = Arc::clone(&latest);
    let ready_tx_for_param = ready_tx.clone();

    // We need a shared cell to carry width/height from param_changed to process.
    let dims = Arc::new(Mutex::new((0u32, 0u32, VideoFormat::Unknown)));
    let dims_param = Arc::clone(&dims);
    let dims_process = Arc::clone(&dims);

    let _listener: StreamListener<()> = stream
        .add_local_listener_with_user_data(())
        .state_changed(|_stream, _data, old, new| {
            tracing::debug!(?old, ?new, "pipewire stream state changed");
        })
        .param_changed(move |_stream, _data, id, param| {
            let Some(pod) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            match parse_format(pod) {
                Ok((media_type, media_subtype)) => {
                    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                        return;
                    }
                    // Extract raw video info (width/height/format) from pod.
                    if let Some((w, h, fmt)) = extract_video_info(pod) {
                        if let Ok(mut g) = dims_param.lock() {
                            *g = (w, h, fmt);
                        }
                        if !ready_sent {
                            let _ = ready_tx_for_param.send(Ok((w, h)));
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "parse_format failed"),
            }
        })
        .process(move |stream, _data| {
            let Some(mut buf) = stream.dequeue_buffer() else {
                return;
            };
            let (w, h, fmt) = match dims_process.lock() {
                Ok(g) => *g,
                Err(_) => return,
            };
            if w == 0 || h == 0 {
                return;
            }
            let datas = buf.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let chunk = data.chunk();
            let stride = chunk.stride() as usize;
            let size = chunk.size() as usize;
            let Some(slice) = data.data() else {
                return;
            };
            let src = &slice[..size.min(slice.len())];

            let mut bgra = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row_start = y * stride;
                if row_start + (w as usize) * 4 > src.len() {
                    break;
                }
                // Re-order bytes into BGRA regardless of source layout.
                for x in 0..w as usize {
                    let base = row_start + x * 4;
                    let (b, g, r, a) = match fmt {
                        VideoFormat::RGBA | VideoFormat::RGBx => (
                            src[base + 2],
                            src[base + 1],
                            src[base],
                            if fmt == VideoFormat::RGBA { src[base + 3] } else { 255 },
                        ),
                        VideoFormat::BGRA | VideoFormat::BGRx => (
                            src[base],
                            src[base + 1],
                            src[base + 2],
                            if fmt == VideoFormat::BGRA { src[base + 3] } else { 255 },
                        ),
                        _ => (src[base], src[base + 1], src[base + 2], 255),
                    };
                    bgra.push(b);
                    bgra.push(g);
                    bgra.push(r);
                    bgra.push(a);
                }
            }

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            if let Ok(mut slot) = latest_for_process.lock() {
                *slot = Some(CapturedFrame {
                    width: w,
                    height: h,
                    data: bgra,
                    timestamp,
                });
            }
        })
        .register()
        .map_err(|e| e.to_string())?;

    // Connect to the node.
    stream
        .connect(
            pw::spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut [],
        )
        .map_err(|e| e.to_string())?;

    // Pump the mainloop; exit on quit signal.
    let mainloop_ref = mainloop.clone();
    std::thread::Builder::new()
        .name("runesh-desktop-pw-quit".into())
        .spawn(move || {
            let _ = quit_rx.recv();
            mainloop_ref.quit();
        })
        .map_err(|e| e.to_string())?;

    mainloop.run();
    // Ensure we unblock the caller even if no ready was sent.
    let _ = ready_tx.send(Err("pipewire mainloop exited before ready".into()));
    Ok(())
}

/// Best-effort extraction of width/height/VideoFormat from a SPA format pod.
fn extract_video_info(pod: &Pod) -> Option<(u32, u32, VideoFormat)> {
    use pw::spa::param::video::VideoInfoRaw;
    let mut info = VideoInfoRaw::default();
    info.parse(pod).ok()?;
    let size = info.size();
    Some((size.width, size.height, info.format()))
}
