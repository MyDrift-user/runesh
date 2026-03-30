//! Linux Wayland screen capture via xdg-desktop-portal (PipeWire ScreenCast).
//!
//! Wayland does not allow applications to capture the screen directly.
//! The only sanctioned method is through the xdg-desktop-portal ScreenCast API,
//! which prompts the user for permission and streams frames via PipeWire.
//!
//! This module provides the D-Bus interaction for setting up a ScreenCast session.
//! Full PipeWire integration requires the `pipewire` crate for frame consumption.
//!
//! ## How it works:
//! 1. Connect to `org.freedesktop.portal.Desktop` on D-Bus
//! 2. Call `org.freedesktop.portal.ScreenCast.CreateSession`
//! 3. Call `SelectSources` to choose displays
//! 4. Call `Start` to begin capture (user sees a permission dialog)
//! 5. Receive a PipeWire node ID
//! 6. Connect to PipeWire and consume video frames
//!
//! ## Current status:
//! This is a stub — full PipeWire integration requires the `pipewire` crate
//! which has system-level dependencies. The architecture is ready for it.

use crate::error::DesktopError;

/// Check if we're running under Wayland.
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// Information about a Wayland ScreenCast session.
#[derive(Debug)]
pub struct WaylandScreenCastSession {
    pub session_handle: String,
    pub pipewire_node_id: u32,
}

/// Request screen capture permission via xdg-desktop-portal.
///
/// This uses D-Bus to communicate with the portal. The user will see
/// a system dialog asking them to select which screen/window to share.
///
/// Returns the PipeWire node ID that can be used to receive frames.
pub async fn request_screencast() -> Result<WaylandScreenCastSession, DesktopError> {
    // Full implementation requires:
    // 1. zbus crate for D-Bus
    // 2. pipewire crate for frame consumption
    //
    // The D-Bus calls would be:
    // - org.freedesktop.portal.ScreenCast.CreateSession
    // - org.freedesktop.portal.ScreenCast.SelectSources (type: MONITOR)
    // - org.freedesktop.portal.ScreenCast.Start
    //
    // The portal returns a PipeWire node ID, and frames are consumed
    // via PipeWire's stream API.
    //
    // For now, return an error indicating this needs PipeWire.

    Err(DesktopError::Unsupported(
        "Wayland screen capture requires PipeWire. \
         Add `pipewire` and `zbus` dependencies for full Wayland support. \
         X11 capture is available as a fallback (XWayland)."
            .into(),
    ))
}

/// Instructions for enabling Wayland capture in consumer projects.
///
/// Add these dependencies to your Cargo.toml:
/// ```toml
/// [target.'cfg(target_os = "linux")'.dependencies]
/// zbus = "5"
/// pipewire = "0.8"
/// ```
///
/// Then implement the PipeWire stream consumer that connects to
/// the node ID returned by `request_screencast()`.
pub fn wayland_setup_instructions() -> &'static str {
    r#"
Wayland Screen Capture Setup:

1. Install system dependencies:
   - Debian/Ubuntu: apt install libpipewire-0.3-dev
   - Fedora: dnf install pipewire-devel
   - Arch: pacman -S pipewire

2. Add Cargo dependencies:
   pipewire = "0.8"
   zbus = "5"

3. The portal will prompt the user to select a screen.
   No special permissions needed — just a user consent dialog.

4. For headless/unattended capture, configure:
   /etc/xdg-desktop-portal/portals.conf
   to use the appropriate backend (wlr, gnome, kde).
"#
}
