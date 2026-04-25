//! Operator input forwarded into the RDP session.
//!
//! We keep the public type minimal — keyboard scan codes, mouse
//! position + button state, scroll deltas — and translate to
//! IronRDP's FastPath builders inside [`crate::session`]. The data
//! channel rumi already runs over WebRTC carries this enum
//! losslessly via serde; consumers re-encode mouse coordinates into
//! the desktop's pixel space before sending.

use serde::{Deserialize, Serialize};

/// One operator input event, in the RDP session's coordinate space.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)] // wired into the IronRDP path in 0.19.x
pub enum InputEvent {
    /// Mouse moved to absolute pixel `(x, y)`. No buttons are
    /// pressed or released by a Move on its own.
    MouseMove { x: i32, y: i32 },
    /// Mouse button pressed. Position is implicit from the most
    /// recent `MouseMove`.
    MouseDown { button: MouseButton },
    /// Mouse button released.
    MouseUp { button: MouseButton },
    /// Vertical scroll. Positive = scroll up. One notch = 120 in
    /// Windows convention; clients are free to pass arbitrary
    /// integers and we'll quantize to whole notches.
    ScrollVertical { delta: i32 },
    /// Horizontal scroll. Positive = scroll right.
    ScrollHorizontal { delta: i32 },
    /// Key pressed. Scancode in PC AT set 1 (the same convention
    /// RDP itself uses). Use `key_extended = true` for keys whose
    /// scancodes are prefixed by `0xE0` in the AT layout (arrow
    /// keys, right Ctrl/Alt, numpad Enter, etc.).
    KeyDown {
        scancode: u16,
        #[serde(default)]
        key_extended: bool,
    },
    /// Key released. Same scancode rules as `KeyDown`.
    KeyUp {
        scancode: u16,
        #[serde(default)]
        key_extended: bool,
    },
    /// Unicode character input. Useful for IME / international
    /// keyboards where a single key produces a code point that
    /// doesn't map cleanly to a scan code.
    Unicode { code_point: u32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// "Back" thumb button.
    X1,
    /// "Forward" thumb button.
    X2,
}
