//! Linux X11 screen capture using XShm (shared memory extension).

use x11rb::connection::Connection;
use x11rb::protocol::shm::{self, ConnectionExt as ShmExt};
use x11rb::protocol::xproto::*;

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

pub struct X11Capturer {
    conn: x11rb::rust_connection::RustConnection,
    screen_num: usize,
    root: Window,
    width: u32,
    height: u32,
    use_shm: bool,
}

impl X11Capturer {
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|e| DesktopError::Capture(format!("X11 connect failed: {e}")))?;

        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        // Check for SHM extension
        let use_shm = conn
            .extension_information(shm::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();

        if use_shm {
            tracing::debug!("X11 SHM extension available, using shared memory capture");
        } else {
            tracing::debug!("X11 SHM not available, using GetImage (slower)");
        }

        Ok(Self {
            conn,
            screen_num,
            root,
            width,
            height,
            use_shm,
        })
    }

    /// Capture using XGetImage (slower but always works).
    fn capture_getimage(&self) -> Result<CapturedFrame, DesktopError> {
        let image = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                self.width as u16,
                self.height as u16,
                !0, // all planes
            )
            .map_err(|e| DesktopError::Capture(format!("GetImage request failed: {e}")))?
            .reply()
            .map_err(|e| DesktopError::Capture(format!("GetImage reply failed: {e}")))?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(CapturedFrame {
            width: self.width,
            height: self.height,
            data: image.data,
            timestamp,
        })
    }
}

impl ScreenCapture for X11Capturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        // For now, use GetImage. SHM requires more setup with shmget/shmat
        // which needs unsafe blocks and OS-level shared memory management.
        self.capture_getimage()
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
