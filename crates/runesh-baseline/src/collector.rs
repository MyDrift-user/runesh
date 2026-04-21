//! System state collector: reads actual OS state for compliance checking.
//!
//! Gathers installed packages, running services, file contents, users,
//! and system settings from the local machine. Works on Linux, macOS,
//! and Windows.

use std::collections::HashMap;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;

use crate::checker::SystemState;

/// Collect the current system state relevant to baseline compliance.
///
/// Reads packages, services, files (from a provided list of paths),
/// users, and settings from the OS.
pub fn collect_system_state(file_paths: &[&str], setting_keys: &[&str]) -> SystemState {
    let mut state = SystemState {
        files: collect_files(file_paths),
        settings: collect_settings(setting_keys),
        ..SystemState::default()
    };

    // Platform-specific collection
    #[cfg(target_os = "linux")]
    {
        state.packages = collect_packages_linux();
        state.services = collect_services_linux();
        state.services_enabled = collect_services_enabled_linux();
        state.users = collect_users_unix();
    }

    #[cfg(target_os = "macos")]
    {
        state.packages = collect_packages_macos();
        state.services = collect_services_macos();
        state.users = collect_users_unix();
    }

    #[cfg(target_os = "windows")]
    {
        state.packages = collect_packages_windows();
        state.services = collect_services_windows();
        state.users = collect_users_windows();
    }

    state
}

fn collect_files(paths: &[&str]) -> HashMap<String, Option<String>> {
    let mut files = HashMap::new();
    for path in paths {
        let p = Path::new(path);
        if p.exists() {
            let content = std::fs::read_to_string(p).ok();
            files.insert(path.to_string(), content);
        }
    }
    files
}

fn collect_settings(keys: &[&str]) -> HashMap<String, String> {
    let mut settings = HashMap::new();

    for key in keys {
        // Linux sysctl
        #[cfg(target_os = "linux")]
        {
            let sysctl_path = format!("/proc/sys/{}", key.replace('.', "/"));
            if let Ok(value) = std::fs::read_to_string(&sysctl_path) {
                settings.insert(key.to_string(), value.trim().to_string());
                continue;
            }
        }

        // Environment variable fallback
        if let Ok(value) = std::env::var(key) {
            settings.insert(key.to_string(), value);
        }
    }

    settings
}

// ---- Linux ----

#[cfg(target_os = "linux")]
fn collect_packages_linux() -> HashMap<String, String> {
    let mut pkgs = HashMap::new();

    // Try dpkg first (Debian/Ubuntu)
    if let Ok(output) = std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Package}\t${Version}\t${Status}\n"])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 && parts[2].contains("installed") {
                pkgs.insert(parts[0].to_string(), parts[1].to_string());
            }
        }
        if !pkgs.is_empty() {
            return pkgs;
        }
    }

    // Try rpm (Fedora/RHEL)
    if let Ok(output) = std::process::Command::new("rpm")
        .args(["-qa", "--queryformat", "%{NAME}\t%{VERSION}-%{RELEASE}\n"])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((name, ver)) = line.split_once('\t') {
                pkgs.insert(name.to_string(), ver.to_string());
            }
        }
        if !pkgs.is_empty() {
            return pkgs;
        }
    }

    // Try pacman (Arch)
    if let Ok(output) = std::process::Command::new("pacman").args(["-Q"]).output() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut parts = line.split_whitespace();
            if let (Some(name), Some(ver)) = (parts.next(), parts.next()) {
                pkgs.insert(name.to_string(), ver.to_string());
            }
        }
    }

    pkgs
}

#[cfg(target_os = "linux")]
fn collect_services_linux() -> HashMap<String, bool> {
    let mut services = HashMap::new();
    if let Ok(output) = std::process::Command::new("systemctl")
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--no-legend",
        ])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let name = parts[0].strip_suffix(".service").unwrap_or(parts[0]);
                let active = parts[2] == "active";
                services.insert(name.to_string(), active);
            }
        }
    }
    services
}

#[cfg(target_os = "linux")]
fn collect_services_enabled_linux() -> HashMap<String, bool> {
    let mut enabled = HashMap::new();
    if let Ok(output) = std::process::Command::new("systemctl")
        .args([
            "list-unit-files",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].strip_suffix(".service").unwrap_or(parts[0]);
                let is_enabled = parts[1] == "enabled";
                enabled.insert(name.to_string(), is_enabled);
            }
        }
    }
    enabled
}

// ---- macOS ----

#[cfg(target_os = "macos")]
fn collect_packages_macos() -> HashMap<String, String> {
    let mut pkgs = HashMap::new();
    if let Ok(output) = std::process::Command::new("brew")
        .args(["list", "--versions"])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut parts = line.split_whitespace();
            if let Some(name) = parts.next() {
                let version = parts.next().unwrap_or("").to_string();
                pkgs.insert(name.to_string(), version);
            }
        }
    }
    pkgs
}

#[cfg(target_os = "macos")]
fn collect_services_macos() -> HashMap<String, bool> {
    let mut services = HashMap::new();
    if let Ok(output) = std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let pid = parts[0].trim();
                let name = parts[2].trim();
                if !name.is_empty() && name != "Label" {
                    services.insert(name.to_string(), pid != "-");
                }
            }
        }
    }
    services
}

// ---- Unix (Linux + macOS) ----

#[cfg(unix)]
fn collect_users_unix() -> HashMap<String, Vec<String>> {
    let mut users = HashMap::new();
    if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 4 {
                let username = parts[0].to_string();
                let uid: u32 = parts[2].parse().unwrap_or(65534);
                // Skip system users (UID < 1000, except root)
                if uid >= 1000 || uid == 0 {
                    users.insert(username, vec![]);
                }
            }
        }
    }
    // Read groups
    if let Ok(group_file) = std::fs::read_to_string("/etc/group") {
        for line in group_file.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 4 {
                let group_name = parts[0];
                let members: Vec<&str> = parts[3].split(',').filter(|s| !s.is_empty()).collect();
                for member in members {
                    if let Some(user_groups) = users.get_mut(member) {
                        user_groups.push(group_name.to_string());
                    }
                }
            }
        }
    }
    users
}

// ---- Windows ----

#[cfg(target_os = "windows")]
fn collect_packages_windows() -> HashMap<String, String> {
    let mut pkgs = HashMap::new();
    // Use winget to list installed packages
    if let Ok(output) = std::process::Command::new("winget")
        .args([
            "list",
            "--disable-interactivity",
            "--accept-source-agreements",
        ])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        // Find separator line
        let lines: Vec<&str> = text.lines().collect();
        if let Some(sep_idx) = lines.iter().position(|l| l.contains("---")) {
            let header = lines.get(sep_idx.wrapping_sub(1)).unwrap_or(&"");
            let id_start = header.find("Id").unwrap_or(0);
            let ver_start = header.find("Version").unwrap_or(0);
            if id_start > 0 && ver_start > 0 {
                for line in &lines[sep_idx + 1..] {
                    if line.len() > ver_start {
                        let id = line
                            .get(id_start..ver_start)
                            .map(|s| s.trim())
                            .unwrap_or("");
                        let ver = line
                            .get(ver_start..)
                            .and_then(|s| s.split_whitespace().next())
                            .unwrap_or("");
                        if !id.is_empty() {
                            pkgs.insert(id.to_string(), ver.to_string());
                        }
                    }
                }
            }
        }
    }
    pkgs
}

#[cfg(target_os = "windows")]
fn collect_services_windows() -> HashMap<String, bool> {
    let mut services = HashMap::new();
    if let Ok(output) = std::process::Command::new("sc")
        .args(["query", "type=", "service", "state=", "all"])
        .creation_flags(0x08000000)
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix("SERVICE_NAME: ") {
                current_name = name.to_string();
            } else if trimmed.starts_with("STATE") && !current_name.is_empty() {
                let running = trimmed.contains("RUNNING");
                services.insert(current_name.clone(), running);
            }
        }
    }
    services
}

#[cfg(target_os = "windows")]
fn collect_users_windows() -> HashMap<String, Vec<String>> {
    let mut users = HashMap::new();
    if let Ok(output) = std::process::Command::new("net")
        .args(["user"])
        .creation_flags(0x08000000)
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut in_list = false;
        for line in text.lines() {
            if line.starts_with("---") {
                in_list = true;
                continue;
            }
            if line.starts_with("The command completed") {
                break;
            }
            if in_list {
                for name in line.split_whitespace() {
                    if !name.is_empty() {
                        users.insert(name.to_string(), vec![]);
                    }
                }
            }
        }
    }
    // Get group memberships
    for username in users.keys().cloned().collect::<Vec<_>>() {
        if let Ok(output) = std::process::Command::new("net")
            .args(["user", &username])
            .creation_flags(0x08000000)
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut in_groups = false;
            let mut groups = Vec::new();
            for line in text.lines() {
                if line.starts_with("Local Group Memberships")
                    || line.starts_with("Global Group memberships")
                {
                    in_groups = true;
                    if let Some(vals) = line.split_once('*') {
                        for g in vals.1.split('*') {
                            let g = g.trim();
                            if !g.is_empty() {
                                groups.push(g.to_string());
                            }
                        }
                    }
                } else if in_groups && line.starts_with(' ') {
                    for g in line.split('*') {
                        let g = g.trim();
                        if !g.is_empty() {
                            groups.push(g.to_string());
                        }
                    }
                } else {
                    in_groups = false;
                }
            }
            if let Some(user_groups) = users.get_mut(&username) {
                *user_groups = groups;
            }
        }
    }
    users
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_files_reads_existing() {
        let state = collect_files(&[if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "/etc/hostname"
        }]);
        // At least the file should be found on any system
        assert!(!state.is_empty() || cfg!(target_os = "macos")); // macOS may not have /etc/hostname
    }

    #[test]
    fn collect_full_state() {
        let state = collect_system_state(&[], &[]);
        // Should not panic on any platform
        // On a real system, packages and services should be populated
        let _ = state.packages.len();
        let _ = state.services.len();
        let _ = state.users.len();
    }

    #[test]
    fn collect_with_settings() {
        let keys = if cfg!(target_os = "linux") {
            vec!["net.ipv4.ip_forward"]
        } else {
            vec!["PATH"]
        };
        let state = collect_system_state(&[], &keys.to_vec());
        // PATH should exist on all platforms, sysctl only on Linux
        if !cfg!(target_os = "linux") {
            assert!(state.settings.contains_key("PATH"));
        }
    }
}
