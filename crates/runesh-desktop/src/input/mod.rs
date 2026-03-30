//! Input injection: send keyboard and mouse events to the remote display.

use crate::error::DesktopError;
use crate::protocol::MouseButton;

/// Input injector trait — implemented per platform.
pub trait InputInjector: Send {
    /// Move the mouse to absolute coordinates.
    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), DesktopError>;

    /// Press or release a mouse button.
    fn mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    ) -> Result<(), DesktopError>;

    /// Press or release a key.
    fn key_event(
        &mut self,
        key_code: u32,
        pressed: bool,
        modifiers: u8,
    ) -> Result<(), DesktopError>;

    /// Scroll the mouse wheel.
    fn scroll(
        &mut self,
        x: i32,
        y: i32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<(), DesktopError>;
}

/// Create an input injector for the current platform.
pub fn create_injector() -> Result<Box<dyn InputInjector>, DesktopError> {
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsInputInjector::new()))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacOsInputInjector::new()))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(x11::X11InputInjector::new()?))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(DesktopError::Unsupported("Input injection not available".into()))
    }
}

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod mpx;
