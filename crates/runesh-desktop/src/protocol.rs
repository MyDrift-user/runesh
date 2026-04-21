//! WebSocket protocol for remote desktop sharing.

use serde::{Deserialize, Serialize};

/// Client → Server desktop messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DesktopRequest {
    /// Start a desktop sharing session.
    StartSession {
        display_id: Option<u32>,
        #[serde(default = "default_quality")]
        quality: Quality,
        #[serde(default = "default_fps")]
        max_fps: u32,
    },
    /// Stop a desktop sharing session.
    StopSession { session_id: String },
    /// Mouse move event (single cursor, backward-compatible).
    MouseMove { x: i32, y: i32, display_id: u32 },
    /// Mouse move with cursor ID (multi-cursor support).
    MouseMoveCursor {
        cursor_id: String,
        x: i32,
        y: i32,
        display_id: u32,
    },
    /// Mouse button with cursor ID (multi-cursor support).
    MouseButtonCursor {
        cursor_id: String,
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    },
    /// Set the multi-cursor control mode.
    SetCursorMode { mode: MultiCursorMode },
    /// Mouse button event.
    MouseButton {
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    },
    /// Keyboard event.
    KeyEvent {
        /// Platform-independent key code.
        key_code: u32,
        pressed: bool,
        /// Modifier flags: bit 0 = shift, bit 1 = ctrl, bit 2 = alt, bit 3 = meta.
        modifiers: u8,
    },
    /// Scroll event.
    Scroll {
        x: i32,
        y: i32,
        delta_x: f32,
        delta_y: f32,
    },
    /// Set clipboard content.
    SetClipboard { content: String },
    /// Select which display to capture.
    SelectDisplay { display_id: u32 },
    /// Change quality settings.
    SetQuality { quality: Quality },
    /// Request an immediate key frame.
    RequestKeyFrame,
    /// List available displays.
    ListDisplays,
}

/// Server → Client desktop messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DesktopResponse {
    /// A captured frame (base64-encoded).
    Frame {
        session_id: String,
        display_id: u32,
        /// Base64-encoded frame data.
        data: String,
        encoding: Encoding,
        width: u32,
        height: u32,
        timestamp: u64,
        /// Whether this is a key frame.
        is_key_frame: bool,
    },
    /// Session started.
    SessionStarted {
        session_id: String,
        display: DisplayInfo,
    },
    /// Session stopped.
    SessionStopped { session_id: String },
    /// Available displays.
    Displays { displays: Vec<DisplayInfo> },
    /// Clipboard update from the remote side.
    ClipboardUpdate { content: String },
    /// Single cursor position update (backward-compatible).
    CursorUpdate { x: i32, y: i32, visible: bool },
    /// Multi-cursor position update — all active cursors in one message.
    CursorPositions { cursors: Vec<CursorState> },
    /// Error response.
    Error { code: String, message: String },
}

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

/// Frame encoding format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Raw RGBA pixels.
    Raw,
    /// PNG compressed.
    Png,
    /// JPEG compressed.
    Jpeg,
    /// Zstd-compressed raw pixels.
    Zstd,
}

/// Quality preset.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
    Lossless,
}

fn default_quality() -> Quality {
    Quality::Medium
}

fn default_fps() -> u32 {
    30
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
