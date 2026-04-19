//! Cross-platform system service installation.
//!
//! Supports Windows (SCM via sc.exe), Linux (systemd), and macOS (launchd).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::AppError;

/// Validate a service name contains only safe characters.
fn validate_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::BadRequest(
            "Service name must be 1-64 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(AppError::BadRequest(
            "Service name must contain only alphanumeric, dash, underscore, or dot".into(),
        ));
    }
    Ok(())
}

/// Validate display name has no control characters or newlines.
fn validate_display_name(name: &str) -> Result<(), AppError> {
    if name.chars().any(|c| c.is_control()) {
        return Err(AppError::BadRequest(
            "Display name must not contain control characters".into(),
        ));
    }
    Ok(())
}

/// XML-escape a string for safe embedding in plist files.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Install a system service that starts at boot.
///
/// - `name`: service name (alphanumeric, dash, underscore, dot only)
/// - `display_name`: human-readable name
/// - `binary`: path to the executable
/// - `args`: command-line arguments
pub fn install_service(
    name: &str,
    display_name: &str,
    binary: &Path,
    args: &[&str],
) -> Result<(), AppError> {
    validate_name(name)?;
    validate_display_name(display_name)?;
    // Validate args contain no control characters or newlines
    for arg in args {
        if arg.chars().any(|c| c.is_control()) {
            return Err(AppError::BadRequest(
                "Arguments must not contain control characters".into(),
            ));
        }
    }
    #[cfg(target_os = "windows")]
    return install_windows(name, display_name, binary, args);

    #[cfg(target_os = "linux")]
    return install_linux(name, display_name, binary, args);

    #[cfg(target_os = "macos")]
    return install_macos(name, display_name, binary, args);

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    Err(AppError::Internal("Unsupported platform".into()))
}

/// Uninstall a system service.
pub fn uninstall_service(name: &str) -> Result<(), AppError> {
    validate_name(name)?;
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("sc").args(["stop", name]).output();
        let out = Command::new("sc")
            .args(["delete", name])
            .output()
            .map_err(|e| AppError::Internal(format!("sc delete failed: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(AppError::Internal(format!("sc delete: {stderr}")));
        }
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let unit = format!("/etc/systemd/system/{name}.service");
        let _ = Command::new("systemctl").args(["stop", name]).output();
        let _ = Command::new("systemctl").args(["disable", name]).output();
        let _ = std::fs::remove_file(&unit);
        let _ = Command::new("systemctl").arg("daemon-reload").output();
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let plist = format!("/Library/LaunchDaemons/{name}.plist");
        let _ = Command::new("launchctl").args(["unload", &plist]).output();
        let _ = std::fs::remove_file(&plist);
        return Ok(());
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    Err(AppError::Internal("Unsupported platform".into()))
}

#[cfg(target_os = "windows")]
fn install_windows(
    name: &str,
    display_name: &str,
    binary: &Path,
    args: &[&str],
) -> Result<(), AppError> {
    let bin_path = binary.to_string_lossy();
    let full_args = if args.is_empty() {
        format!("\"{}\"", bin_path)
    } else {
        format!("\"{}\" {}", bin_path, args.join(" "))
    };

    let out = Command::new("sc")
        .args([
            "create",
            name,
            &format!("binPath= {full_args}"),
            &format!("DisplayName= {display_name}"),
            "start=",
            "auto",
        ])
        .output()
        .map_err(|e| AppError::Internal(format!("sc create failed: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.contains("already exists") {
            return Err(AppError::Internal(format!("sc create: {stderr}")));
        }
    }

    // Configure restart on failure
    let _ = Command::new("sc")
        .args([
            "failure",
            name,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .output();

    let _ = Command::new("sc").args(["start", name]).output();

    tracing::info!(name, "Windows service installed");
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_linux(
    name: &str,
    display_name: &str,
    binary: &Path,
    args: &[&str],
) -> Result<(), AppError> {
    let bin_path = binary.to_string_lossy();
    let exec_start = if args.is_empty() {
        bin_path.to_string()
    } else {
        format!("{} {}", bin_path, args.join(" "))
    };

    let unit = format!(
        "[Unit]\nDescription={display_name}\nAfter=network-online.target\nWants=network-online.target\nBefore=display-manager.service\n\n[Service]\nType=simple\nExecStart={exec_start}\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=multi-user.target\n"
    );

    let unit_path = format!("/etc/systemd/system/{name}.service");
    std::fs::write(&unit_path, &unit)
        .map_err(|e| AppError::Internal(format!("Failed to write unit file: {e}")))?;

    let _ = Command::new("systemctl").arg("daemon-reload").output();
    let _ = Command::new("systemctl").args(["enable", name]).output();
    let _ = Command::new("systemctl").args(["start", name]).output();

    tracing::info!(name, "systemd service installed");
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_macos(
    name: &str,
    display_name: &str,
    binary: &Path,
    args: &[&str],
) -> Result<(), AppError> {
    let bin_path = xml_escape(&binary.to_string_lossy());
    let args_xml = args
        .iter()
        .map(|a| format!("    <string>{}</string>", xml_escape(a)))
        .collect::<Vec<_>>()
        .join("\n");

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{name}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin_path}</string>
{args_xml}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>"#
    );

    let plist_path = format!("/Library/LaunchDaemons/{name}.plist");
    std::fs::write(&plist_path, &plist)
        .map_err(|e| AppError::Internal(format!("Failed to write plist: {e}")))?;

    let _ = Command::new("launchctl")
        .args(["load", &plist_path])
        .output();

    tracing::info!(name, "launchd daemon installed");
    Ok(())
}
