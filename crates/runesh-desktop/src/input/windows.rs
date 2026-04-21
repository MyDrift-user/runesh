//! Windows input injection using SendInput API.
//!
//! We intentionally construct INPUT via `Default::default()` and then assign
//! the tagged-union fields individually because the union cannot be populated
//! with a single struct literal.
#![allow(clippy::field_reassign_with_default)]

use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::InputInjector;
use crate::error::DesktopError;
use crate::protocol::MouseButton;

pub struct WindowsInputInjector {
    screen_width: i32,
    screen_height: i32,
}

impl Default for WindowsInputInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsInputInjector {
    pub fn new() -> Self {
        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        Self {
            screen_width,
            screen_height,
        }
    }

    /// Convert screen coordinates to absolute mouse coordinates (0-65535 range).
    fn to_absolute(&self, x: i32, y: i32) -> (i32, i32) {
        let abs_x = ((x as f64 / self.screen_width as f64) * 65535.0) as i32;
        let abs_y = ((y as f64 / self.screen_height as f64) * 65535.0) as i32;
        (abs_x, abs_y)
    }

    fn send_mouse(
        &self,
        dx: i32,
        dy: i32,
        mouse_data: u32,
        flags: MOUSE_EVENT_FLAGS,
    ) -> Result<(), DesktopError> {
        let mut input = INPUT::default();
        input.r#type = INPUT_MOUSE;
        input.Anonymous.mi = MOUSEINPUT {
            dx,
            dy,
            mouseData: mouse_data,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            return Err(DesktopError::Input("SendInput failed".into()));
        }
        Ok(())
    }

    fn send_key(&self, vk: u16, flags: KEYBD_EVENT_FLAGS) -> Result<(), DesktopError> {
        let mut input = INPUT::default();
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk),
            wScan: 0,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            return Err(DesktopError::Input("SendInput failed".into()));
        }
        Ok(())
    }
}

impl InputInjector for WindowsInputInjector {
    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), DesktopError> {
        let (abs_x, abs_y) = self.to_absolute(x, y);
        self.send_mouse(abs_x, abs_y, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)
    }

    fn mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    ) -> Result<(), DesktopError> {
        self.mouse_move(x, y)?;

        let flags = match (button, pressed) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            _ => return Ok(()),
        };

        self.send_mouse(0, 0, 0, flags)
    }

    fn key_event(
        &mut self,
        key_code: u32,
        pressed: bool,
        _modifiers: u8,
    ) -> Result<(), DesktopError> {
        let flags = if pressed {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        };
        self.send_key(key_code as u16, flags)
    }

    fn scroll(&mut self, x: i32, y: i32, _delta_x: f32, delta_y: f32) -> Result<(), DesktopError> {
        self.mouse_move(x, y)?;
        let wheel_delta = (delta_y * 120.0) as u32;
        self.send_mouse(0, 0, wheel_delta, MOUSEEVENTF_WHEEL)
    }
}
