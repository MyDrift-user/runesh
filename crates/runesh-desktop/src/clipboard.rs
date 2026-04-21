//! Cross-platform clipboard sharing using the arboard crate.

use serde::{Deserialize, Serialize};

/// Which way clipboard data is allowed to flow across a session.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardDirection {
    /// Clipboard is not synced in either direction.
    #[default]
    None,
    /// Only the host clipboard is pushed to the viewer.
    HostToViewer,
    /// Only the viewer clipboard is written to the host.
    ViewerToHost,
    /// Bidirectional sync.
    Bidirectional,
}

impl ClipboardDirection {
    pub fn allows_host_to_viewer(&self) -> bool {
        matches!(self, Self::HostToViewer | Self::Bidirectional)
    }
    pub fn allows_viewer_to_host(&self) -> bool {
        matches!(self, Self::ViewerToHost | Self::Bidirectional)
    }
}

/// Clipboard sync settings. Used by the session manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardSettings {
    pub direction: ClipboardDirection,
    /// Max bytes per clipboard payload. Default 1 MiB.
    pub max_bytes: usize,
    /// Polling rate for host-to-viewer sync in milliseconds.
    pub poll_rate_ms: u64,
    /// Max viewer-to-host writes per second.
    pub write_rate_per_sec: u32,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            direction: ClipboardDirection::None,
            max_bytes: 1024 * 1024,
            poll_rate_ms: 500,
            write_rate_per_sec: 10,
        }
    }
}

#[cfg(feature = "clipboard")]
mod clipboard_impl {
    use arboard::Clipboard;

    use crate::error::DesktopError;

    /// Clipboard manager for bidirectional clipboard sharing.
    pub struct ClipboardManager {
        clipboard: Clipboard,
        last_content: String,
    }

    impl ClipboardManager {
        pub fn new() -> Result<Self, DesktopError> {
            let clipboard = Clipboard::new()
                .map_err(|e| DesktopError::Internal(format!("Clipboard init failed: {e}")))?;

            Ok(Self {
                clipboard,
                last_content: String::new(),
            })
        }

        /// Get the current clipboard text content.
        pub fn get_text(&mut self) -> Result<String, DesktopError> {
            self.clipboard
                .get_text()
                .map_err(|e| DesktopError::Internal(format!("Clipboard get failed: {e}")))
        }

        /// Set the clipboard text content.
        pub fn set_text(&mut self, text: &str) -> Result<(), DesktopError> {
            self.clipboard
                .set_text(text)
                .map_err(|e| DesktopError::Internal(format!("Clipboard set failed: {e}")))?;
            self.last_content = text.to_string();
            Ok(())
        }

        /// Check if clipboard content has changed since last check.
        /// Returns Some(new_content) if changed, None otherwise.
        pub fn poll_change(&mut self) -> Option<String> {
            if let Ok(current) = self.get_text()
                && current != self.last_content
            {
                self.last_content = current.clone();
                return Some(current);
            }
            None
        }
    }
}

#[cfg(feature = "clipboard")]
pub use clipboard_impl::ClipboardManager;
