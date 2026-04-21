//! System tray setup for Tauri v2.
//!
//! Provides a builder for common tray icon patterns:
//! show/hide window, connect/disconnect, quit.
//!
//! Tray menu IDs are forwarded to the frontend as events. Even though this
//! module validates IDs against a strict regex at registration, frontend
//! handlers MUST continue to treat incoming IDs as untrusted: Tauri's IPC
//! boundary still exists and the tray plumbing is shared.

use tauri::{
    AppHandle, Emitter, Manager,
    menu::{IsMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
};

/// Menu item IDs for the default tray menu.
pub const TRAY_SHOW: &str = "show";
pub const TRAY_QUIT: &str = "quit";

/// Validate a tray menu ID against `^[a-z][a-z0-9_-]{0,63}$`.
pub fn valid_tray_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

/// Errors returned by tray construction.
#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    #[error("invalid tray menu id '{0}': must match ^[a-z][a-z0-9_-]{{0,63}}$")]
    InvalidId(String),
    #[error("tauri error: {0}")]
    Tauri(String),
}

/// Create a basic tray icon with Show and Quit actions.
///
/// Menu IDs are validated at registration. Invalid IDs are rejected before
/// any Tauri resources are created.
///
/// Usage:
/// ```ignore
/// use runesh_tauri::tray::setup_tray;
///
/// fn main() {
///     tauri::Builder::default()
///         .setup(|app| {
///             setup_tray(app.handle(), include_bytes!("../icons/icon.png"), &[
///                 ("Show", "show"),
///                 ("Quit", "quit"),
///             ])?;
///             Ok(())
///         })
///         .run(tauri::generate_context!())
///         .expect("error running app");
/// }
/// ```
pub fn setup_tray(
    app: &AppHandle,
    icon_bytes: &[u8],
    items: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate every id before doing any work.
    for (_label, id) in items {
        if !valid_tray_id(id) {
            return Err(Box::new(TrayError::InvalidId((*id).to_string())));
        }
    }

    let mut menu_items = Vec::new();
    for (label, id) in items {
        menu_items.push(MenuItem::with_id(app, *id, *label, true, None::<&str>)?);
    }

    let refs: Vec<&dyn IsMenuItem<_>> =
        menu_items.iter().map(|m| m as &dyn IsMenuItem<_>).collect();
    let menu = Menu::with_items(app, &refs)?;

    let icon = tauri::image::Image::from_bytes(icon_bytes)?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                id => {
                    // Custom menu items -- emit event for the frontend to
                    // handle. IDs passed through here were validated at
                    // registration time, but frontend handlers must still
                    // treat them as untrusted strings.
                    let _ = app.emit("tray-action", id);
                }
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ids() {
        assert!(valid_tray_id("show"));
        assert!(valid_tray_id("quit"));
        assert!(valid_tray_id("a1_b-c"));
        assert!(valid_tray_id(&"a".repeat(64)));
    }

    #[test]
    fn invalid_ids() {
        assert!(!valid_tray_id(""));
        assert!(!valid_tray_id("Show"));
        assert!(!valid_tray_id("1show"));
        assert!(!valid_tray_id("show panel"));
        assert!(!valid_tray_id(&"a".repeat(65)));
        assert!(!valid_tray_id("../evil"));
        // Trailing hyphen is allowed by the regex but uncommon in practice.
        assert!(valid_tray_id("show-"));
    }
}
