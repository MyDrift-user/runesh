//! Wire protocol for remote desktop sharing.
//!
//! There are three surfaces:
//!
//! 1. **Signaling** (WebSocket, JSON) — [`SignalRequest`] / [`SignalResponse`].
//!    Minimal message set to bootstrap a WebRTC peer connection: SDP offer/answer
//!    and trickled ICE candidates. After the peer connection is up, everything
//!    else flows over WebRTC.
//!
//! 2. **Control** (WebRTC DataChannel, JSON) — [`ControlRequest`] / [`ControlResponse`].
//!    Session lifecycle (start/stop), quality changes, display selection, clipboard,
//!    cursor state.
//!
//! 3. **Input** (WebRTC DataChannel, binary) — see [`input_binary`]. Compact
//!    byte layout for mouse/key/scroll events to avoid JSON overhead at 1kHz.
//!
//! Frames themselves are carried out of band on a WebRTC RTP video track,
//! and audio on a separate RTP audio track. Neither traverses this protocol.

use serde::{Deserialize, Serialize};

// ── Signaling (WebSocket, JSON) ───────────────────────────────────────────

/// Client → Server signaling messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalRequest {
    /// First message the client sends. Authenticates the WebSocket and
    /// declares what the client wants.
    Auth {
        token: String,
        /// Optional label for the connecting user (used for cursor colouring etc.).
        display_name: Option<String>,
    },
    /// Ask the server to begin a session and emit an SDP offer.
    Start {
        display_id: Option<u32>,
        #[serde(default = "default_quality")]
        quality: Quality,
        #[serde(default = "default_fps")]
        max_fps: u32,
        /// Whether the client wants an audio track in the offer.
        #[serde(default = "default_true")]
        audio: bool,
    },
    /// SDP answer from the client in response to the server's offer.
    Answer { sdp: String },
    /// A trickled ICE candidate from the client.
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    /// Ask the server to list available displays.
    ListDisplays,
    /// Politely close the session and tear down the peer connection.
    Hangup,
}

/// Server → Client signaling messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalResponse {
    /// Authentication succeeded.
    AuthOk {
        cursor_id: String,
        cursor_color: String,
    },
    /// Server's SDP offer, built after [`SignalRequest::Start`].
    Offer { session_id: String, sdp: String },
    /// Server-side ICE candidate to hand to the client's peer connection.
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    /// The peer connection moved to the `Connected` state.
    PeerConnected { session_id: String },
    /// The peer connection failed or was closed.
    PeerClosed { reason: String },
    /// Display enumeration result.
    Displays { displays: Vec<DisplayInfo> },
    /// Error response.
    Error { code: String, message: String },
}

// ── Control messages (DataChannel, JSON) ──────────────────────────────────

/// Client → Server control messages (after WebRTC is connected).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Change quality on the fly.
    SetQuality { quality: Quality },
    /// Switch the capture source to another display.
    SelectDisplay { display_id: u32 },
    /// Ask the encoder for a key frame right now (e.g. after tab switch).
    RequestKeyFrame,
    /// Set the multi-cursor control mode.
    SetCursorMode { mode: MultiCursorMode },
    /// Push a clipboard payload from the viewer to the host.
    SetClipboard { content: String },
    /// Ping for RTT measurement.
    Ping { nonce: u64 },
}

/// Server → Client control messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlResponse {
    /// Confirmation that a mutating request was applied.
    Ack { id: String },
    /// Clipboard update from the host.
    ClipboardUpdate { content: String },
    /// Multi-cursor position update — all active cursors in one message.
    CursorPositions { cursors: Vec<CursorState> },
    /// Periodic statistics (bitrate, fps, queue depth) for the client UI.
    Stats {
        bitrate_kbps: u32,
        fps: u32,
        width: u32,
        height: u32,
        queue_depth: u32,
    },
    /// Pong echoing a previous ping.
    Pong { nonce: u64 },
    /// Error response carried over the DataChannel.
    Error { code: String, message: String },
}

// ── Binary input protocol (DataChannel, binary) ───────────────────────────

/// Compact binary encoding of mouse/keyboard/scroll events.
///
/// ## Frame format
///
/// Every frame begins with a single opcode byte, followed by opcode-specific
/// fixed-size payload. All integers are little-endian.
///
/// | opcode | name        | total bytes | payload layout                                                                    |
/// |--------|-------------|-------------|-----------------------------------------------------------------------------------|
/// | `0x01` | MouseMove   | 9           | `i32 x, i32 y`                                                                    |
/// | `0x02` | MouseButton | 11          | `u8 button, u8 pressed (0/1), i32 x, i32 y`                                       |
/// | `0x03` | KeyEvent    | 7           | `u32 key_code, u8 pressed (0/1), u8 modifiers`                                    |
/// | `0x04` | Scroll      | 17          | `i32 x, i32 y, f32 delta_x, f32 delta_y`                                          |
///
/// Button codes: `1=Left, 2=Right, 3=Middle, 4=Back, 5=Forward`.
pub mod input_binary {
    use super::MouseButton;

    pub const OP_MOUSE_MOVE: u8 = 0x01;
    pub const OP_MOUSE_BUTTON: u8 = 0x02;
    pub const OP_KEY_EVENT: u8 = 0x03;
    pub const OP_SCROLL: u8 = 0x04;

    /// Decoded input event.
    #[derive(Debug, Clone)]
    pub enum InputEvent {
        MouseMove {
            x: i32,
            y: i32,
        },
        MouseButton {
            button: MouseButton,
            pressed: bool,
            x: i32,
            y: i32,
        },
        KeyEvent {
            key_code: u32,
            pressed: bool,
            modifiers: u8,
        },
        Scroll {
            x: i32,
            y: i32,
            delta_x: f32,
            delta_y: f32,
        },
    }

    /// Decode a single event from a byte slice. Returns `Err` for malformed
    /// or truncated frames. Never panics.
    pub fn decode(bytes: &[u8]) -> Result<InputEvent, &'static str> {
        if bytes.is_empty() {
            return Err("empty input frame");
        }
        match bytes[0] {
            OP_MOUSE_MOVE => {
                if bytes.len() < 9 {
                    return Err("mouse_move payload truncated");
                }
                let x = i32::from_le_bytes(bytes[1..5].try_into().unwrap());
                let y = i32::from_le_bytes(bytes[5..9].try_into().unwrap());
                Ok(InputEvent::MouseMove { x, y })
            }
            OP_MOUSE_BUTTON => {
                if bytes.len() < 11 {
                    return Err("mouse_button payload truncated");
                }
                let button = decode_button(bytes[1])?;
                let pressed = bytes[2] != 0;
                let x = i32::from_le_bytes(bytes[3..7].try_into().unwrap());
                let y = i32::from_le_bytes(bytes[7..11].try_into().unwrap());
                Ok(InputEvent::MouseButton {
                    button,
                    pressed,
                    x,
                    y,
                })
            }
            OP_KEY_EVENT => {
                if bytes.len() < 7 {
                    return Err("key_event payload truncated");
                }
                let key_code = u32::from_le_bytes(bytes[1..5].try_into().unwrap());
                let pressed = bytes[5] != 0;
                let modifiers = bytes[6];
                Ok(InputEvent::KeyEvent {
                    key_code,
                    pressed,
                    modifiers,
                })
            }
            OP_SCROLL => {
                if bytes.len() < 17 {
                    return Err("scroll payload truncated");
                }
                let x = i32::from_le_bytes(bytes[1..5].try_into().unwrap());
                let y = i32::from_le_bytes(bytes[5..9].try_into().unwrap());
                let delta_x = f32::from_le_bytes(bytes[9..13].try_into().unwrap());
                let delta_y = f32::from_le_bytes(bytes[13..17].try_into().unwrap());
                Ok(InputEvent::Scroll {
                    x,
                    y,
                    delta_x,
                    delta_y,
                })
            }
            _ => Err("unknown input opcode"),
        }
    }

    fn decode_button(code: u8) -> Result<MouseButton, &'static str> {
        match code {
            1 => Ok(MouseButton::Left),
            2 => Ok(MouseButton::Right),
            3 => Ok(MouseButton::Middle),
            4 => Ok(MouseButton::Back),
            5 => Ok(MouseButton::Forward),
            _ => Err("unknown mouse button code"),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn button_code(b: MouseButton) -> u8 {
            match b {
                MouseButton::Left => 1,
                MouseButton::Right => 2,
                MouseButton::Middle => 3,
                MouseButton::Back => 4,
                MouseButton::Forward => 5,
            }
        }

        #[test]
        fn roundtrip_mouse_move() {
            let mut buf = vec![OP_MOUSE_MOVE];
            buf.extend_from_slice(&123_i32.to_le_bytes());
            buf.extend_from_slice(&(-456_i32).to_le_bytes());
            match decode(&buf).unwrap() {
                InputEvent::MouseMove { x, y } => {
                    assert_eq!(x, 123);
                    assert_eq!(y, -456);
                }
                _ => panic!("wrong variant"),
            }
        }

        #[test]
        fn roundtrip_mouse_button() {
            let mut buf = vec![OP_MOUSE_BUTTON, button_code(MouseButton::Right), 1];
            buf.extend_from_slice(&10_i32.to_le_bytes());
            buf.extend_from_slice(&20_i32.to_le_bytes());
            match decode(&buf).unwrap() {
                InputEvent::MouseButton {
                    button,
                    pressed,
                    x,
                    y,
                } => {
                    assert!(matches!(button, MouseButton::Right));
                    assert!(pressed);
                    assert_eq!(x, 10);
                    assert_eq!(y, 20);
                }
                _ => panic!("wrong variant"),
            }
        }

        #[test]
        fn rejects_truncated() {
            assert!(decode(&[OP_MOUSE_MOVE, 1, 2, 3]).is_err());
            assert!(decode(&[]).is_err());
            assert!(decode(&[0xFF]).is_err());
        }
    }
}

// ── Shared types (used by signaling, control, and frame metadata) ────────

/// Display information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
    pub scale_factor: f32,
}

/// Quality preset. Drives the video encoder's target bitrate and tuning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
    Lossless,
}

impl Quality {
    /// Target bitrate in kilobits per second for this quality level at 1080p.
    /// Scaled linearly for larger resolutions.
    pub fn target_kbps_for(self, width: u32, height: u32) -> u32 {
        let base: u64 = match self {
            Quality::Low => 1_000,
            Quality::Medium => 3_500,
            Quality::High => 8_000,
            Quality::Lossless => 20_000,
        };
        let pixels = (width as u64) * (height as u64);
        let base_pixels = 1920u64 * 1080;
        let scaled = base * pixels / base_pixels;
        scaled.clamp(500, 60_000) as u32
    }
}

fn default_quality() -> Quality {
    Quality::Medium
}

fn default_fps() -> u32 {
    30
}

fn default_true() -> bool {
    true
}

/// Mouse button enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Modifier key flags.
pub mod modifiers {
    pub const SHIFT: u8 = 1 << 0;
    pub const CTRL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const META: u8 = 1 << 3;
}

// ── Multi-Cursor Types ────────────────────────────────────────────────────

/// State of a single cursor in a multi-cursor session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorState {
    /// Unique identifier for this cursor (typically the connection/user ID).
    pub cursor_id: String,
    /// Human-readable label (e.g., "Tech Support", "John D.").
    pub label: String,
    /// Absolute X position on the display.
    pub x: i32,
    /// Absolute Y position on the display.
    pub y: i32,
    /// Hex color for this cursor (e.g., "#FF4444").
    pub color: String,
    /// Cursor shape.
    pub shape: CursorShape,
    /// Whether this cursor is currently visible.
    pub visible: bool,
    /// True if this is the local OS cursor (not software-rendered).
    pub is_local: bool,
    /// Whether this cursor currently has input focus (can click/type).
    pub has_focus: bool,
}

/// Cursor shape types for multi-cursor rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum CursorShape {
    #[default]
    Arrow,
    Hand,
    IBeam,
    Crosshair,
    ResizeNs,
    ResizeEw,
    ResizeNesw,
    ResizeNwse,
    Wait,
    NotAllowed,
}

/// Multi-cursor control mode — how input conflicts are resolved.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MultiCursorMode {
    /// Both cursors move freely; last click wins input focus.
    #[default]
    Collaborative,
    /// Remote technician has exclusive input control.
    TechControl,
    /// Local user has exclusive input control; tech can only observe.
    UserControl,
    /// Tech must request control; user approves via prompt.
    RequestControl,
}

/// Default colors assigned to cursors in order of connection.
pub const CURSOR_COLORS: &[&str] = &[
    "#4A90D9", // Blue (local user)
    "#E74C3C", // Red (first remote)
    "#2ECC71", // Green
    "#F39C12", // Orange
    "#9B59B6", // Purple
    "#1ABC9C", // Teal
];
