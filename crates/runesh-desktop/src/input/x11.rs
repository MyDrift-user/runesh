//! Linux X11 input injection using XTest extension.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::xtest::ConnectionExt as XtestExt;

use super::InputInjector;
use crate::error::DesktopError;
use crate::protocol::MouseButton;

pub struct X11InputInjector {
    conn: x11rb::rust_connection::RustConnection,
    screen_num: usize,
}

impl X11InputInjector {
    pub fn new() -> Result<Self, DesktopError> {
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|e| DesktopError::Input(format!("X11 connect failed: {e}")))?;

        // Verify XTest extension is available
        conn.extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .map_err(|e| DesktopError::Input(format!("XTest query failed: {e}")))?
            .ok_or_else(|| DesktopError::Input("XTest extension not available".into()))?;

        Ok(Self { conn, screen_num })
    }
}

impl InputInjector for X11InputInjector {
    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), DesktopError> {
        let screen = &self.conn.setup().roots[self.screen_num];

        self.conn
            .warp_pointer(
                x11rb::NONE, // src_window
                screen.root,
                0,
                0, // src position (ignored)
                0,
                0, // src size (ignored)
                x as i16,
                y as i16,
            )
            .map_err(|e| DesktopError::Input(format!("WarpPointer failed: {e}")))?;

        self.conn
            .flush()
            .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

        Ok(())
    }

    fn mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    ) -> Result<(), DesktopError> {
        // Move to position first
        self.mouse_move(x, y)?;

        let x11_button: u8 = match button {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 3,
            MouseButton::Back => 8,
            MouseButton::Forward => 9,
        };

        let event_type = if pressed {
            2 // ButtonPress
        } else {
            3 // ButtonRelease
        };

        self.conn
            .xtest_fake_input(event_type, x11_button, 0, x11rb::NONE, 0, 0, 0)
            .map_err(|e| DesktopError::Input(format!("FakeInput button failed: {e}")))?;

        self.conn
            .flush()
            .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

        Ok(())
    }

    fn key_event(
        &mut self,
        key_code: u32,
        pressed: bool,
        _modifiers: u8,
    ) -> Result<(), DesktopError> {
        let event_type = if pressed {
            2 // KeyPress
        } else {
            3 // KeyRelease
        };

        self.conn
            .xtest_fake_input(event_type, key_code as u8, 0, x11rb::NONE, 0, 0, 0)
            .map_err(|e| DesktopError::Input(format!("FakeInput key failed: {e}")))?;

        self.conn
            .flush()
            .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

        Ok(())
    }

    fn scroll(&mut self, x: i32, y: i32, _delta_x: f32, delta_y: f32) -> Result<(), DesktopError> {
        self.mouse_move(x, y)?;

        // X11 scroll is button 4 (up) and 5 (down)
        let (button, clicks) = if delta_y > 0.0 {
            (4u8, delta_y.abs() as u32)
        } else {
            (5u8, delta_y.abs() as u32)
        };

        let clicks = clicks.max(1).min(10);

        for _ in 0..clicks {
            // Press
            self.conn
                .xtest_fake_input(2, button, 0, x11rb::NONE, 0, 0, 0)
                .map_err(|e| DesktopError::Input(format!("FakeInput scroll press: {e}")))?;
            // Release
            self.conn
                .xtest_fake_input(3, button, 0, x11rb::NONE, 0, 0, 0)
                .map_err(|e| DesktopError::Input(format!("FakeInput scroll release: {e}")))?;
        }

        self.conn
            .flush()
            .map_err(|e| DesktopError::Input(format!("Flush failed: {e}")))?;

        Ok(())
    }
}
