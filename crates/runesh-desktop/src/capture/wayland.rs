//! Linux Wayland screen capture via xdg-desktop-portal + PipeWire.
//!
//! On Wayland, apps can't touch the framebuffer directly. The sanctioned path is:
//!
//! 1. Ask `org.freedesktop.portal.ScreenCast` (via `ashpd`) for a ScreenCast
//!    session; the user sees a system-level picker dialog.
//! 2. The portal returns a PipeWire node id + a file descriptor.
//! 3. We open a PipeWire stream on that node and receive frames in one of
//!    several raw video formats.
//! 4. Each buffer is copied into a [`CapturedFrame`] in BGRA layout (what the
//!    rest of the crate expects) and handed back.
//!
//! This module compiles only on Linux with the `wayland` feature on. `ashpd`
//! and `pipewire` are pulled in conditionally.

use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{
    CreateSessionOptions, CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions,
    SourceType, StartCastOptions,
};
use pipewire as pw;
use pw::spa;
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::Pod;
use pw::stream::StreamFlags;

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

/// Detect whether we're running under Wayland.
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
}

/// Quit signal sent from the capturer to the PipeWire thread.
struct Terminate;

/// Shared state mutated by the PipeWire callbacks.
#[derive(Default)]
struct SharedState {
    width: u32,
    height: u32,
    format: Option<VideoFormat>,
    latest: Option<CapturedFrame>,
}

/// Live Wayland screen capture handle.
pub struct WaylandCapturer {
    width: u32,
    height: u32,
    shared: Arc<Mutex<SharedState>>,
    /// Sender used to stop the PipeWire mainloop; dropping the capturer stops it.
    quit_tx: Option<pw::channel::Sender<Terminate>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WaylandCapturer {
    fn drop(&mut self) {
        if let Some(tx) = self.quit_tx.take() {
            let _ = tx.send(Terminate);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl WaylandCapturer {
    /// Request a ScreenCast session from xdg-desktop-portal and start reading
    /// frames from the returned PipeWire node.
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DesktopError::Capture(format!("wayland runtime: {e}")))?;

        let (fd, node_id, portal_w, portal_h) = rt
            .block_on(open_portal_session(display_id))
            .map_err(|e| DesktopError::Capture(format!("xdg-desktop-portal: {e}")))?;

        let shared = Arc::new(Mutex::new(SharedState {
            width: portal_w,
            height: portal_h,
            ..Default::default()
        }));
        let shared_for_thread = Arc::clone(&shared);

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(u32, u32), String>>();
        let (quit_tx, quit_rx) = pw::channel::channel::<Terminate>();

        let thread = std::thread::Builder::new()
            .name("runesh-desktop-pipewire".into())
            .spawn(move || {
                if let Err(e) = run_pw_loop(fd, node_id, shared_for_thread, ready_tx, quit_rx) {
                    tracing::error!(error = %e, "pipewire mainloop exited with error");
                }
            })
            .map_err(|e| DesktopError::Capture(format!("spawn pipewire thread: {e}")))?;

        let (w, h) = match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(d)) => d,
            Ok(Err(msg)) => return Err(DesktopError::Capture(msg)),
            Err(_) => {
                return Err(DesktopError::Capture(
                    "pipewire stream did not become ready in time".into(),
                ));
            }
        };

        Ok(Self {
            width: w.max(portal_w),
            height: h.max(portal_h),
            shared,
            quit_tx: Some(quit_tx),
            thread: Some(thread),
        })
    }
}

impl ScreenCapture for WaylandCapturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        let guard = self
            .shared
            .lock()
            .map_err(|_| DesktopError::Capture("wayland state lock poisoned".into()))?;
        match guard.latest.as_ref() {
            Some(f) => Ok(CapturedFrame {
                width: f.width,
                height: f.height,
                data: f.data.clone(),
                timestamp: f.timestamp,
            }),
            None => Err(DesktopError::Capture(
                "no pipewire frame available yet".into(),
            )),
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

// ── Portal session setup ──────────────────────────────────────────────────

async fn open_portal_session(
    display_id: u32,
) -> Result<(std::os::unix::io::OwnedFd, u32, u32, u32), String> {
    let proxy = Screencast::new().await.map_err(|e| e.to_string())?;

    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|e| e.to_string())?;

    let select = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::Embedded)
        .set_sources(enumflags2::BitFlags::from(SourceType::Monitor))
        .set_persist_mode(PersistMode::DoNot);
    let _ = proxy
        .select_sources(&session, select)
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;

    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;

    let streams_vec = streams.streams();
    let stream = streams_vec
        .get(display_id as usize)
        .or_else(|| streams_vec.first())
        .ok_or_else(|| "no ScreenCast streams returned".to_string())?;
    let node_id = stream.pipe_wire_node_id();
    let (w, h) = stream
        .size()
        .map(|(w, h)| (w as u32, h as u32))
        .unwrap_or((0, 0));

    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(|e| e.to_string())?;

    Ok((fd, node_id, w, h))
}

// ── PipeWire mainloop ─────────────────────────────────────────────────────

fn run_pw_loop(
    fd: std::os::unix::io::OwnedFd,
    node_id: u32,
    shared: Arc<Mutex<SharedState>>,
    ready_tx: std::sync::mpsc::Sender<Result<(u32, u32), String>>,
    quit_rx: pw::channel::Receiver<Terminate>,
) -> Result<(), String> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;

    let stream = pw::stream::StreamRc::new(
        core,
        "runesh-desktop-wayland-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;

    let shared_param = Arc::clone(&shared);
    let shared_process = Arc::clone(&shared);
    let ready_tx_param = ready_tx.clone();
    let ready_sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ready_sent_clone = Arc::clone(&ready_sent);

    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(|_stream, _user_data, old, new| {
            tracing::debug!(?old, ?new, "pipewire stream state");
        })
        .param_changed(move |_stream, _user_data, id, param| {
            let Some(pod) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let (media_type, media_subtype) = match format_utils::parse_format(pod) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "parse_format failed");
                    return;
                }
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut info = VideoInfoRaw::default();
            if let Err(e) = info.parse(pod) {
                tracing::warn!(error = %e, "video format parse failed");
                return;
            }
            let size = info.size();
            let (w, h) = (size.width, size.height);
            let fmt = info.format();
            if let Ok(mut s) = shared_param.lock() {
                s.width = w;
                s.height = h;
                s.format = Some(fmt);
            }
            if !ready_sent_clone.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let _ = ready_tx_param.send(Ok((w, h)));
            }
        })
        .process(move |stream, _user_data| {
            let Some(mut buf) = stream.dequeue_buffer() else {
                return;
            };
            let (w, h, fmt) = {
                let Ok(s) = shared_process.lock() else {
                    return;
                };
                let Some(fmt) = s.format else {
                    return;
                };
                (s.width, s.height, fmt)
            };
            if w == 0 || h == 0 {
                return;
            }
            let datas = buf.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let stride = data.chunk().stride() as usize;
            let size = data.chunk().size() as usize;
            let Some(slice) = data.data() else {
                return;
            };
            let src = &slice[..size.min(slice.len())];
            let bgra = to_bgra(w, h, stride, fmt, src);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if let Ok(mut s) = shared_process.lock() {
                s.latest = Some(CapturedFrame {
                    width: w,
                    height: h,
                    data: bgra,
                    timestamp,
                });
            }
        })
        .register()
        .map_err(|e| e.to_string())?;

    // Build an EnumFormat pod listing the raw video formats we accept.
    let params_bytes = build_format_params();
    let pod = Pod::from_bytes(&params_bytes).ok_or_else(|| "pod deserialise failed".to_string())?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| e.to_string())?;

    // Cross-thread quit: when a Terminate lands on the channel, call mainloop.quit().
    let _quit_source = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    mainloop.run();
    if !ready_sent.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = ready_tx.send(Err("pipewire exited before format was negotiated".into()));
    }
    Ok(())
}

/// Serialize an `EnumFormat` pod that lists raw BGRA/RGBx/… at any size and framerate.
fn build_format_params() -> Vec<u8> {
    let obj = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: vec![
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaType,
                Id,
                MediaType::Video
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaSubtype,
                Id,
                MediaSubtype::Raw
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                VideoFormat::BGRA,
                VideoFormat::BGRA,
                VideoFormat::BGRx,
                VideoFormat::RGBA,
                VideoFormat::RGBx,
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                pw::spa::utils::Rectangle {
                    width: 1920,
                    height: 1080,
                },
                pw::spa::utils::Rectangle {
                    width: 1,
                    height: 1,
                },
                pw::spa::utils::Rectangle {
                    width: 8192,
                    height: 8192,
                },
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoFramerate,
                Choice,
                Range,
                Fraction,
                pw::spa::utils::Fraction { num: 30, denom: 1 },
                pw::spa::utils::Fraction { num: 0, denom: 1 },
                pw::spa::utils::Fraction { num: 144, denom: 1 },
            ),
        ],
    };
    pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .expect("pod serialize")
    .0
    .into_inner()
}

/// Reformat a source buffer into BGRA regardless of incoming layout.
fn to_bgra(width: u32, height: u32, stride: usize, fmt: VideoFormat, src: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut out = Vec::with_capacity(w * h * 4);
    let stride = stride.max(w * 4);
    for y in 0..h {
        let row_start = y * stride;
        if row_start + w * 4 > src.len() {
            break;
        }
        for x in 0..w {
            let base = row_start + x * 4;
            let (b, g, r, a) = match fmt {
                VideoFormat::RGBA => (src[base + 2], src[base + 1], src[base], src[base + 3]),
                VideoFormat::RGBx => (src[base + 2], src[base + 1], src[base], 255),
                VideoFormat::BGRA => (src[base], src[base + 1], src[base + 2], src[base + 3]),
                VideoFormat::BGRx => (src[base], src[base + 1], src[base + 2], 255),
                _ => (src[base], src[base + 1], src[base + 2], 255),
            };
            out.push(b);
            out.push(g);
            out.push(r);
            out.push(a);
        }
    }
    out
}
