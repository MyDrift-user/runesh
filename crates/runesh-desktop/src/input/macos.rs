//! macOS input injection using Core Graphics events.

use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use super::InputInjector;
use crate::error::DesktopError;
use crate::protocol::MouseButton;

pub struct MacOsInputInjector {
    event_source: CGEventSource,
}

impl MacOsInputInjector {
    pub fn new() -> Self {
        let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .expect("Failed to create CGEventSource");

        Self { event_source }
    }
}

impl InputInjector for MacOsInputInjector {
    fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), DesktopError> {
        let point = CGPoint::new(x as f64, y as f64);
        let event = CGEvent::new_mouse_event(
            self.event_source.clone(),
            CGEventType::MouseMoved,
            point,
            CGMouseButton::Left,
        )
        .map_err(|_| DesktopError::Input("Failed to create mouse move event".into()))?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    ) -> Result<(), DesktopError> {
        let point = CGPoint::new(x as f64, y as f64);
        let (event_type, cg_button) = match (button, pressed) {
            (MouseButton::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (MouseButton::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (MouseButton::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (MouseButton::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (MouseButton::Middle, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (MouseButton::Middle, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
            _ => return Ok(()),
        };

        let event = CGEvent::new_mouse_event(
            self.event_source.clone(),
            event_type,
            point,
            cg_button,
        )
        .map_err(|_| DesktopError::Input("Failed to create mouse button event".into()))?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn key_event(
        &mut self,
        key_code: u32,
        pressed: bool,
        _modifiers: u8,
    ) -> Result<(), DesktopError> {
        let event = CGEvent::new_keyboard_event(
            self.event_source.clone(),
            key_code as u16,
            pressed,
        )
        .map_err(|_| DesktopError::Input("Failed to create keyboard event".into()))?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn scroll(
        &mut self,
        _x: i32,
        _y: i32,
        _delta_x: f32,
        delta_y: f32,
    ) -> Result<(), DesktopError> {
        let event = CGEvent::new_scroll_event(
            self.event_source.clone(),
            core_graphics::event::ScrollEventUnit::PIXEL,
            1, // wheel count
            delta_y as i32,
            0,
            0,
        )
        .map_err(|_| DesktopError::Input("Failed to create scroll event".into()))?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
