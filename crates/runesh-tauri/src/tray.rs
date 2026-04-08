//! System tray setup for Tauri v2.
//!
//! Provides a builder for common tray icon patterns:
//! show/hide window, connect/disconnect, quit.

use tauri::{
    menu::{IsMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};

/// Menu item IDs for the default tray menu.
pub const TRAY_SHOW: &str = "show";
pub const TRAY_QUIT: &str = "quit";

/// Create a basic tray icon with Show and Quit actions.
///
/// The icon bytes should be a PNG image (e.g. `include_bytes!("../icons/icon.png")`).
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
    let mut menu_items = Vec::new();
    for (label, id) in items {
        menu_items.push(MenuItem::with_id(app, *id, *label, true, None::<&str>)?);
    }

    let refs: Vec<&dyn IsMenuItem<_>> = menu_items
        .iter()
        .map(|m| m as &dyn IsMenuItem<_>)
        .collect();
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
                    // Custom menu items - emit event for the frontend to handle
                    let _ = app.emit("tray-action", id);
                }
            }
        })
        .build(app)?;

    Ok(())
}
