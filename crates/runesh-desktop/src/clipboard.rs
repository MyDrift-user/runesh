//! Cross-platform clipboard sharing using the arboard crate.

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
            if let Ok(current) = self.get_text() {
                if current != self.last_content {
                    self.last_content = current.clone();
                    return Some(current);
                }
            }
            None
        }
    }
}

#[cfg(feature = "clipboard")]
pub use clipboard_impl::ClipboardManager;
