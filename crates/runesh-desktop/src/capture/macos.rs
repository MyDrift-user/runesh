//! macOS screen capture using Core Graphics (CGDisplayCreateImage).

use core_graphics::display::{CGDisplay, CGPoint, CGRect, CGSize};

use super::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

/// Query `CGSessionCopyCurrentDictionary` for the
/// `CGSSessionScreenIsLocked` key. Returns true if the user's session is
/// locked. Fails closed (returns false) if the API is unavailable; the
/// caller should treat that as "not locked" rather than proceed.
fn is_session_locked() -> bool {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};

    #[allow(unsafe_code)]
    unsafe {
        // The symbol is part of the public CoreGraphics interface. Declare
        // it here rather than pulling a new crate just for one function.
        #[allow(improper_ctypes)]
        unsafe extern "C" {
            fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
        }
        let raw = CGSessionCopyCurrentDictionary();
        if raw.is_null() {
            return false;
        }
        let dict: CFDictionary<CFString, CFType> =
            CFDictionary::wrap_under_create_rule(raw as *const _);
        let key = CFString::new("CGSSessionScreenIsLocked");
        match dict.find(&key) {
            Some(v) => {
                // Value is a CFBoolean or CFNumber depending on macOS version.
                if let Ok(b) = v.downcast::<CFBoolean>() {
                    bool::from(b)
                } else if let Ok(n) = v.downcast::<CFNumber>() {
                    n.to_i64().map(|x| x != 0).unwrap_or(false)
                } else {
                    false
                }
            }
            None => false,
        }
    }
}

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
        if is_session_locked() {
            return Err(DesktopError::Capture(
                "macOS user session is locked; capture suppressed".into(),
            ));
        }
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
