//! Multi-cursor tracking, sprite rendering, and frame compositing.
//!
//! Provides software-rendered overlay cursors for multi-user remote desktop
//! sessions. Each connected user gets a colored cursor with a label, composited
//! onto captured frames before encoding.

use std::collections::HashMap;

use crate::capture::CapturedFrame;
use crate::protocol::{CursorShape, CursorState, MultiCursorMode, CURSOR_COLORS};

// ── Cursor Tracker ────────────────────────────────────────────────────────

/// Tracks all active cursors and manages input focus.
#[derive(Debug, Clone)]
pub struct CursorTracker {
    /// All active cursors keyed by cursor_id.
    cursors: HashMap<String, CursorState>,
    /// Which cursor currently owns input focus (can inject OS-level input).
    active_cursor: Option<String>,
    /// How input conflicts between cursors are resolved.
    pub mode: MultiCursorMode,
    /// Counter for assigning colors.
    next_color_index: usize,
}

impl CursorTracker {
    pub fn new(mode: MultiCursorMode) -> Self {
        Self {
            cursors: HashMap::new(),
            active_cursor: None,
            mode,
            next_color_index: 0,
        }
    }

    /// Register a new cursor for a connected user.
    /// Returns the assigned color.
    pub fn add_cursor(&mut self, cursor_id: &str, label: &str, is_local: bool) -> String {
        let color = CURSOR_COLORS[self.next_color_index % CURSOR_COLORS.len()].to_string();
        self.next_color_index += 1;

        let cursor = CursorState {
            cursor_id: cursor_id.to_string(),
            label: label.to_string(),
            x: 0,
            y: 0,
            color: color.clone(),
            shape: CursorShape::default(),
            visible: true,
            is_local,
            has_focus: is_local, // local cursor starts with focus
        };

        if is_local && self.active_cursor.is_none() {
            self.active_cursor = Some(cursor_id.to_string());
        }

        self.cursors.insert(cursor_id.to_string(), cursor);
        color
    }

    /// Remove a cursor when a user disconnects.
    pub fn remove_cursor(&mut self, cursor_id: &str) {
        self.cursors.remove(cursor_id);
        if self.active_cursor.as_deref() == Some(cursor_id) {
            // Transfer focus to first remaining cursor
            self.active_cursor = self.cursors.keys().next().cloned();
            if let Some(ref active) = self.active_cursor {
                if let Some(c) = self.cursors.get_mut(active) {
                    c.has_focus = true;
                }
            }
        }
    }

    /// Update a cursor's position.
    pub fn update_position(&mut self, cursor_id: &str, x: i32, y: i32) {
        if let Some(cursor) = self.cursors.get_mut(cursor_id) {
            cursor.x = x;
            cursor.y = y;
        }
    }

    /// Update a cursor's shape.
    pub fn update_shape(&mut self, cursor_id: &str, shape: CursorShape) {
        if let Some(cursor) = self.cursors.get_mut(cursor_id) {
            cursor.shape = shape;
        }
    }

    /// Transfer input focus to a cursor (e.g., on click in Collaborative mode).
    pub fn set_focus(&mut self, cursor_id: &str) {
        // Remove focus from all
        for cursor in self.cursors.values_mut() {
            cursor.has_focus = false;
        }
        // Set focus on the specified cursor
        if let Some(cursor) = self.cursors.get_mut(cursor_id) {
            cursor.has_focus = true;
            self.active_cursor = Some(cursor_id.to_string());
        }
    }

    /// Check if the given cursor should inject OS-level input.
    pub fn should_inject_input(&self, cursor_id: &str) -> bool {
        match self.mode {
            MultiCursorMode::Collaborative => {
                self.active_cursor.as_deref() == Some(cursor_id)
            }
            MultiCursorMode::TechControl => {
                // Remote cursors (non-local) have control
                self.cursors
                    .get(cursor_id)
                    .is_some_and(|c| !c.is_local)
            }
            MultiCursorMode::UserControl => {
                // Only local cursor has control
                self.cursors
                    .get(cursor_id)
                    .is_some_and(|c| c.is_local)
            }
            MultiCursorMode::RequestControl => {
                self.active_cursor.as_deref() == Some(cursor_id)
            }
        }
    }

    /// Get all cursor states for broadcasting.
    pub fn all_cursors(&self) -> Vec<CursorState> {
        self.cursors.values().cloned().collect()
    }

    /// Get non-local cursors (the ones that need software rendering).
    pub fn remote_cursors(&self) -> Vec<&CursorState> {
        self.cursors.values().filter(|c| !c.is_local).collect()
    }

    /// Check if any remote cursors are active.
    pub fn has_remote_cursors(&self) -> bool {
        self.cursors.values().any(|c| !c.is_local && c.visible)
    }
}

// ── Cursor Sprite Rendering ───────────────────────────────────────────────

/// A pre-rendered cursor sprite as RGBA pixels.
pub struct CursorSprite {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data.
    pub data: Vec<u8>,
}

/// Generate a colored arrow cursor sprite (24x24 pixels).
pub fn generate_arrow_sprite(hex_color: &str) -> CursorSprite {
    let (r, g, b) = parse_hex_color(hex_color);
    let width = 24u32;
    let height = 24u32;
    let mut data = vec![0u8; (width * height * 4) as usize];

    // Arrow shape defined as rows of (start_col, end_col) fills
    // Classic arrow cursor pointing top-left
    let arrow_shape: &[(u32, u32)] = &[
        (0, 1),   // row 0
        (0, 2),   // row 1
        (0, 3),   // row 2
        (0, 4),   // row 3
        (0, 5),   // row 4
        (0, 6),   // row 5
        (0, 7),   // row 6
        (0, 8),   // row 7
        (0, 9),   // row 8
        (0, 10),  // row 9
        (0, 11),  // row 10
        (0, 12),  // row 11
        (0, 6),   // row 12: narrows back
        (0, 3), (5, 7),   // row 13 (two segments handled below)
        (0, 3), (6, 8),   // row 14
        (0, 2), (7, 9),   // row 15
        (0, 2), (8, 10),  // row 16
        (0, 1), (9, 11),  // row 17
        (0, 1), (10, 11), // row 18
    ];

    // Fill arrow with color + black outline
    for (row, spans) in arrow_shape.iter().enumerate() {
        let y = row as u32;
        if y >= height {
            break;
        }
        // Outline pixel (black) at start
        if spans.0 < width {
            set_pixel(&mut data, width, spans.0, y, 0, 0, 0, 255);
        }
        // Fill pixels with color
        for x in (spans.0 + 1)..spans.1.min(width) {
            set_pixel(&mut data, width, x, y, r, g, b, 230);
        }
        // Outline pixel at end
        if spans.1 > 0 && spans.1 < width {
            set_pixel(&mut data, width, spans.1 - 1, y, 0, 0, 0, 255);
        }
    }

    // Left edge outline
    for y in 0..19u32.min(height) {
        set_pixel(&mut data, width, 0, y, 0, 0, 0, 255);
    }

    CursorSprite {
        width,
        height,
        data,
    }
}

/// Generate a label bitmap as RGBA pixels.
/// Simple 5x7 pixel font for uppercase letters + digits.
pub fn generate_label_sprite(text: &str, hex_color: &str) -> CursorSprite {
    let (r, g, b) = parse_hex_color(hex_color);
    let char_width = 6u32; // 5px + 1px spacing
    let char_height = 9u32; // 7px + 2px padding
    let max_chars = text.len().min(16);
    let width = (max_chars as u32 * char_width) + 4; // 2px padding each side
    let height = char_height + 4; // 2px padding top/bottom

    let mut data = vec![0u8; (width * height * 4) as usize];

    // Draw background pill (semi-transparent dark)
    for y in 0..height {
        for x in 0..width {
            set_pixel(&mut data, width, x, y, 30, 30, 30, 180);
        }
    }

    // Draw border
    for x in 0..width {
        set_pixel(&mut data, width, x, 0, r, g, b, 255);
        set_pixel(&mut data, width, x, height - 1, r, g, b, 255);
    }
    for y in 0..height {
        set_pixel(&mut data, width, 0, y, r, g, b, 255);
        set_pixel(&mut data, width, width - 1, y, r, g, b, 255);
    }

    // Draw text (simple 5x7 bitmap font for ASCII)
    for (i, ch) in text.chars().take(max_chars).enumerate() {
        let glyph = get_glyph(ch);
        let x_offset = 2 + (i as u32 * char_width);
        let y_offset = 3u32;

        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) != 0 {
                    let px = x_offset + col;
                    let py = y_offset + row as u32;
                    if px < width && py < height {
                        set_pixel(&mut data, width, px, py, 255, 255, 255, 255);
                    }
                }
            }
        }
    }

    CursorSprite {
        width,
        height,
        data,
    }
}

// ── Frame Compositing ─────────────────────────────────────────────────────

/// Composite all remote cursors onto a captured frame (in-place BGRA mutation).
pub fn composite_cursors(frame: &mut CapturedFrame, tracker: &CursorTracker) {
    if !tracker.has_remote_cursors() {
        return;
    }

    for cursor in tracker.remote_cursors() {
        if !cursor.visible {
            continue;
        }

        // Draw cursor arrow sprite
        let arrow = generate_arrow_sprite(&cursor.color);
        blit_sprite_bgra(frame, cursor.x, cursor.y, &arrow);

        // Draw label below the cursor
        if !cursor.label.is_empty() {
            let label = generate_label_sprite(&cursor.label, &cursor.color);
            blit_sprite_bgra(
                frame,
                cursor.x + 4,
                cursor.y + arrow.height as i32 + 2,
                &label,
            );
        }
    }
}

/// Alpha-blend an RGBA sprite onto a BGRA frame at the given position.
fn blit_sprite_bgra(frame: &mut CapturedFrame, x: i32, y: i32, sprite: &CursorSprite) {
    let fw = frame.width as i32;
    let fh = frame.height as i32;

    for sy in 0..sprite.height as i32 {
        let fy = y + sy;
        if fy < 0 || fy >= fh {
            continue;
        }

        for sx in 0..sprite.width as i32 {
            let fx = x + sx;
            if fx < 0 || fx >= fw {
                continue;
            }

            let sprite_idx = ((sy * sprite.width as i32 + sx) * 4) as usize;
            let frame_idx = ((fy * fw + fx) * 4) as usize;

            if sprite_idx + 3 >= sprite.data.len() || frame_idx + 3 >= frame.data.len() {
                continue;
            }

            // Sprite is RGBA, frame is BGRA
            let sr = sprite.data[sprite_idx] as u32;
            let sg = sprite.data[sprite_idx + 1] as u32;
            let sb = sprite.data[sprite_idx + 2] as u32;
            let sa = sprite.data[sprite_idx + 3] as u32;

            if sa == 0 {
                continue;
            }

            if sa == 255 {
                // Opaque — direct write (RGBA → BGRA)
                frame.data[frame_idx] = sb as u8;     // B
                frame.data[frame_idx + 1] = sg as u8;  // G
                frame.data[frame_idx + 2] = sr as u8;  // R
                frame.data[frame_idx + 3] = 255;       // A
            } else {
                // Alpha blend
                let da = 255 - sa;
                let db = frame.data[frame_idx] as u32;
                let dg = frame.data[frame_idx + 1] as u32;
                let dr = frame.data[frame_idx + 2] as u32;

                frame.data[frame_idx] = ((sb * sa + db * da) / 255) as u8;
                frame.data[frame_idx + 1] = ((sg * sa + dg * da) / 255) as u8;
                frame.data[frame_idx + 2] = ((sr * sa + dr * da) / 255) as u8;
                frame.data[frame_idx + 3] = 255;
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn set_pixel(data: &mut [u8], width: u32, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    let idx = ((y * width + x) * 4) as usize;
    if idx + 3 < data.len() {
        data[idx] = r;
        data[idx + 1] = g;
        data[idx + 2] = b;
        data[idx + 3] = a;
    }
}

fn parse_hex_color(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        (r, g, b)
    } else {
        (255, 255, 255)
    }
}

/// Minimal 5x7 bitmap font. Returns 7 bytes, each representing one row (MSB = left).
fn get_glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b11100, 0b10010, 0b10001, 0b10001, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b01110, 0b00001],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10010, 0b10001, 0b10001],
        'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        ' ' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        _   => [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111], // box
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_tracker_add_remove() {
        let mut tracker = CursorTracker::new(MultiCursorMode::Collaborative);
        tracker.add_cursor("local", "User", true);
        tracker.add_cursor("remote", "Tech", false);

        assert_eq!(tracker.all_cursors().len(), 2);
        assert!(tracker.has_remote_cursors());

        tracker.remove_cursor("remote");
        assert_eq!(tracker.all_cursors().len(), 1);
        assert!(!tracker.has_remote_cursors());
    }

    #[test]
    fn test_collaborative_focus() {
        let mut tracker = CursorTracker::new(MultiCursorMode::Collaborative);
        tracker.add_cursor("local", "User", true);
        tracker.add_cursor("remote", "Tech", false);

        // Local starts with focus
        assert!(tracker.should_inject_input("local"));
        assert!(!tracker.should_inject_input("remote"));

        // Remote clicks — takes focus
        tracker.set_focus("remote");
        assert!(!tracker.should_inject_input("local"));
        assert!(tracker.should_inject_input("remote"));
    }

    #[test]
    fn test_tech_control_mode() {
        let mut tracker = CursorTracker::new(MultiCursorMode::TechControl);
        tracker.add_cursor("local", "User", true);
        tracker.add_cursor("remote", "Tech", false);

        assert!(!tracker.should_inject_input("local"));
        assert!(tracker.should_inject_input("remote"));
    }

    #[test]
    fn test_sprite_generation() {
        let sprite = generate_arrow_sprite("#FF0000");
        assert_eq!(sprite.width, 24);
        assert_eq!(sprite.height, 24);
        assert_eq!(sprite.data.len(), (24 * 24 * 4) as usize);
    }

    #[test]
    fn test_label_generation() {
        let label = generate_label_sprite("Tech", "#FF0000");
        assert!(label.width > 0);
        assert!(label.height > 0);
    }

    #[test]
    fn test_hex_color_parsing() {
        assert_eq!(parse_hex_color("#FF0000"), (255, 0, 0));
        assert_eq!(parse_hex_color("#00FF00"), (0, 255, 0));
        assert_eq!(parse_hex_color("0000FF"), (0, 0, 255));
    }
}
