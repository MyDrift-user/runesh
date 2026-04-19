//! Screen capture abstraction with platform-specific backends.

use crate::error::DesktopError;

/// A captured frame: raw BGRA pixel data.
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// Raw pixel data in BGRA format.
    pub data: Vec<u8>,
    pub timestamp: u64,
}

/// Screen capture trait — implemented per platform.
pub trait ScreenCapture: Send {
    /// Capture a single frame from the display.
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError>;

    /// Get the display dimensions.
    fn dimensions(&self) -> (u32, u32);
}

/// Create a screen capturer for the specified display.
pub fn create_capturer(display_id: u32) -> Result<Box<dyn ScreenCapture>, DesktopError> {
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::DxgiCapturer::new(display_id)?))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::CgCapturer::new(display_id)?))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(x11::X11Capturer::new(display_id)?))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = display_id;
        Err(DesktopError::Unsupported(
            "No screen capture on this platform".into(),
        ))
    }
}

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;
