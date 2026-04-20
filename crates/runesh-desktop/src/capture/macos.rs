//! macOS screen capture using Core Graphics (CGDisplayCreateImage).

use core_graphics::display::{CGDisplay, CGPoint, CGRect, CGSize};

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

pub struct CgCapturer {
    display_id: u32,
    width: u32,
    height: u32,
}

impl CgCapturer {
    pub fn new(display_id: u32) -> Result<Self, DesktopError> {
        let display = CGDisplay::new(display_id);

        if !display.is_active() {
            return Err(DesktopError::DisplayNotFound(display_id));
        }

        let bounds = display.bounds();
        let width = bounds.size.width as u32;
        let height = bounds.size.height as u32;

        Ok(Self {
            display_id,
            width,
            height,
        })
    }
}

impl ScreenCapture for CgCapturer {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        let display = CGDisplay::new(self.display_id);

        // Capture the display
        let image = CGDisplay::image(&display)
            .ok_or_else(|| DesktopError::Capture("CGDisplayCreateImage returned null".into()))?;

        let width = image.width() as u32;
        let height = image.height() as u32;
        let bytes_per_row = image.bytes_per_row();
        let raw_data = image.data();
        let pixel_data = raw_data.bytes();

        // Convert from CGImage format (potentially with padding) to packed BGRA
        let expected_row = (width * 4) as usize;
        let mut data = Vec::with_capacity((width * height * 4) as usize);

        for row in 0..height as usize {
            let start = row * bytes_per_row;
            let end = start + expected_row;
            if end <= pixel_data.len() {
                data.extend_from_slice(&pixel_data[start..end]);
            }
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.width = width;
        self.height = height;

        Ok(CapturedFrame {
            width,
            height,
            data,
            timestamp,
        })
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
