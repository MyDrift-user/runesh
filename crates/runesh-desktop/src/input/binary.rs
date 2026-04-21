//! Glue between the binary input wire format and the platform [`InputInjector`].
//!
//! The decoder lives in [`crate::protocol::input_binary`]; this file just
//! dispatches decoded events into the injector.

use super::InputInjector;
use crate::error::DesktopError;
use crate::protocol::input_binary::{InputEvent, decode};

/// Feed one binary DataChannel payload into the injector.
pub fn dispatch(bytes: &[u8], injector: &mut dyn InputInjector) -> Result<(), DesktopError> {
    let event =
        decode(bytes).map_err(|e| DesktopError::Input(format!("binary input decode: {e}")))?;
    match event {
        InputEvent::MouseMove { x, y } => injector.mouse_move(x, y),
        InputEvent::MouseButton {
            button,
            pressed,
            x,
            y,
        } => injector.mouse_button(button, pressed, x, y),
        InputEvent::KeyEvent {
            key_code,
            pressed,
            modifiers,
        } => injector.key_event(key_code, pressed, modifiers),
        InputEvent::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => injector.scroll(x, y, delta_x, delta_y),
    }
}
