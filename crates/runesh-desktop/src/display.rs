//! Display enumeration — detect available monitors.

use crate::error::DesktopError;
use crate::protocol::DisplayInfo;

/// Enumerate all available displays.
pub fn enumerate_displays() -> Result<Vec<DisplayInfo>, DesktopError> {
    #[cfg(target_os = "windows")]
    return enumerate_displays_windows();

    #[cfg(target_os = "macos")]
    return enumerate_displays_macos();

    #[cfg(target_os = "linux")]
    return enumerate_displays_linux();

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Err(DesktopError::Unsupported("Unknown platform".into()))
}

#[cfg(target_os = "windows")]
fn enumerate_displays_windows() -> Result<Vec<DisplayInfo>, DesktopError> {
    use windows::Win32::Graphics::Dxgi::*;

    let mut displays = Vec::new();

    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()
            .map_err(|e| DesktopError::Capture(format!("CreateDXGIFactory1 failed: {e}")))?;

        let mut adapter_idx = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
            let mut output_idx = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_idx) {
                let desc = output.GetDesc()
                    .map_err(|e| DesktopError::Capture(format!("GetDesc failed: {e}")))?;

                let name = String::from_utf16_lossy(
                    &desc.DeviceName[..desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len())]
                );

                let rect = desc.DesktopCoordinates;
                let width = (rect.right - rect.left) as u32;
                let height = (rect.bottom - rect.top) as u32;

                displays.push(DisplayInfo {
                    id: displays.len() as u32,
                    name,
                    width,
                    height,
                    x: rect.left,
                    y: rect.top,
                    is_primary: displays.is_empty(),
                    scale_factor: 1.0,
                });

                output_idx += 1;
            }
            adapter_idx += 1;
        }
    }

    if displays.is_empty() {
        displays.push(DisplayInfo {
            id: 0,
            name: "Primary".into(),
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            is_primary: true,
            scale_factor: 1.0,
        });
    }

    Ok(displays)
}

#[cfg(target_os = "macos")]
fn enumerate_displays_macos() -> Result<Vec<DisplayInfo>, DesktopError> {
    use core_graphics::display::CGDisplay;

    let active_displays = CGDisplay::active_displays()
        .map_err(|e| DesktopError::Capture(format!("Failed to get displays: {e:?}")))?;

    let displays = active_displays
        .iter()
        .enumerate()
        .map(|(i, &display_id)| {
            let display = CGDisplay::new(display_id);
            let bounds = display.bounds();

            DisplayInfo {
                id: display_id,
                name: format!("Display {}", i + 1),
                width: bounds.size.width as u32,
                height: bounds.size.height as u32,
                x: bounds.origin.x as i32,
                y: bounds.origin.y as i32,
                is_primary: display.is_main(),
                scale_factor: 1.0,
            }
        })
        .collect();

    Ok(displays)
}

#[cfg(target_os = "linux")]
fn enumerate_displays_linux() -> Result<Vec<DisplayInfo>, DesktopError> {
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as RandrExt;

    let (conn, screen_num) = x11rb::connect(None)
        .map_err(|e| DesktopError::Capture(format!("X11 connect failed: {e}")))?;

    let screen = &conn.setup().roots[screen_num];
    let mut displays = Vec::new();

    // Try RandR for multi-monitor info
    if let Ok(resources) = conn.randr_get_screen_resources_current(screen.root) {
        if let Ok(resources) = resources.reply() {
            for (i, &output) in resources.outputs.iter().enumerate() {
                if let Ok(output_info) = conn.randr_get_output_info(output, resources.config_timestamp) {
                    if let Ok(info) = output_info.reply() {
                        if info.connection == x11rb::protocol::randr::Connection::CONNECTED {
                            if info.crtc != 0 {
                                if let Ok(crtc_info) = conn.randr_get_crtc_info(info.crtc, resources.config_timestamp) {
                                    if let Ok(crtc) = crtc_info.reply() {
                                        let name = String::from_utf8_lossy(&info.name).to_string();
                                        displays.push(DisplayInfo {
                                            id: i as u32,
                                            name,
                                            width: crtc.width as u32,
                                            height: crtc.height as u32,
                                            x: crtc.x as i32,
                                            y: crtc.y as i32,
                                            is_primary: i == 0,
                                            scale_factor: 1.0,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: use root window dimensions
    if displays.is_empty() {
        displays.push(DisplayInfo {
            id: 0,
            name: "Screen 0".into(),
            width: screen.width_in_pixels as u32,
            height: screen.height_in_pixels as u32,
            x: 0,
            y: 0,
            is_primary: true,
            scale_factor: 1.0,
        });
    }

    Ok(displays)
}
