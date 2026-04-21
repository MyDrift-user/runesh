//! Linux platform backend.
//!
//! Design: prefer canonical system tools (`hostnamectl`, `timedatectl`,
//! `useradd`, `nmcli`, `chsh`, `gsettings`, `xrandr`) invoked with
//! `Command::args` - never a shell interpolation. These tools ship in
//! distribution-standard places, have stable command lines, and are what
//! the system administrator would use by hand. The applier reads current
//! state via the native files under `/etc` and `/proc` when possible.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Output;

use async_trait::async_trait;
use tokio::process::Command;

use crate::{
    ApplyReport, ConfigError, ConfigSpec, DesktopConfig, DisplayConfig, HostnameConfig,
    IpAddressing, MonitorConfig, NetworkConfig, SectionOutcome, ShellConfig, TimezoneConfig,
    UserConfig, UsersConfig, WallpaperFit,
    platform::{ConfigApplier, PlatformBackend, run_spec},
};

pub(crate) struct LinuxApplier;

impl LinuxApplier {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ConfigApplier for LinuxApplier {
    async fn apply(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, false).await
    }

    async fn dry_run(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, true).await
    }
}

#[async_trait]
impl PlatformBackend for LinuxApplier {
    async fn hostname(
        &self,
        cfg: &HostnameConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_hostname(&cfg.name)?;

        let current = read_trimmed("/proc/sys/kernel/hostname")
            .await
            .unwrap_or_default();
        let persisted = read_trimmed("/etc/hostname").await.unwrap_or_default();
        if current == cfg.name && persisted == cfg.name {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }

        // hostnamectl drives systemd-hostnamed; it updates both the running
        // kernel value and `/etc/hostname` atomically.
        run_checked("hostnamectl", &["set-hostname", &cfg.name]).await?;
        Ok(SectionOutcome::Changed)
    }

    async fn timezone(
        &self,
        cfg: &TimezoneConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_iana_tz(&cfg.iana)?;

        let current = current_timezone_linux().await;
        if current.as_deref() == Some(cfg.iana.as_str()) {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }

        run_checked("timedatectl", &["set-timezone", &cfg.iana]).await?;
        Ok(SectionOutcome::Changed)
    }

    async fn users(&self, cfg: &UsersConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        let mut changed = false;

        for user in &cfg.present {
            validate_username(&user.name)?;
            if user_exists_linux(&user.name).await {
                if reconcile_user_linux(user, dry).await? {
                    changed = true;
                }
            } else {
                if dry {
                    changed = true;
                    continue;
                }
                create_user_linux(user).await?;
                changed = true;
            }
        }

        for name in &cfg.absent {
            validate_username(name)?;
            if !user_exists_linux(name).await {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }
            run_checked("userdel", &["-r", name]).await?;
            changed = true;
        }

        Ok(if changed {
            if dry {
                SectionOutcome::WouldChange
            } else {
                SectionOutcome::Changed
            }
        } else {
            SectionOutcome::AlreadyCurrent
        })
    }

    async fn network(&self, cfg: &NetworkConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        if !command_exists("nmcli").await {
            return Err(ConfigError::NotSupported(
                "NetworkManager (nmcli) is required for network reconciliation on Linux".into(),
            ));
        }

        let mut changed = false;

        for adapter in &cfg.adapters {
            let nmcli_out = run_capture(
                "nmcli",
                &[
                    "-t",
                    "-f",
                    "GENERAL.CONNECTION,IP4.ADDRESS",
                    "device",
                    "show",
                    &adapter.id,
                ],
            )
            .await?;
            let current_addr = parse_nmcli_ipv4(&nmcli_out);
            let desired_addr = match &adapter.addressing {
                IpAddressing::Dhcp => None,
                IpAddressing::Static {
                    address, prefix, ..
                } => Some(format!("{address}/{prefix}")),
            };
            let matches = current_addr == desired_addr;
            let enabled_ok = adapter.enabled;
            if matches && enabled_ok {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }

            match &adapter.addressing {
                IpAddressing::Dhcp => {
                    run_checked(
                        "nmcli",
                        &["device", "modify", &adapter.id, "ipv4.method", "auto"],
                    )
                    .await?;
                }
                IpAddressing::Static {
                    address,
                    prefix,
                    gateway,
                } => {
                    let cidr = format!("{address}/{prefix}");
                    let mut args: Vec<&str> = vec![
                        "device",
                        "modify",
                        &adapter.id,
                        "ipv4.method",
                        "manual",
                        "ipv4.addresses",
                        &cidr,
                    ];
                    let gw = gateway.as_deref().unwrap_or("");
                    if !gw.is_empty() {
                        args.extend_from_slice(&["ipv4.gateway", gw]);
                    }
                    run_checked("nmcli", &args).await?;
                }
            }

            if let Some(mtu) = adapter.mtu {
                let mtu_str = mtu.to_string();
                run_checked(
                    "nmcli",
                    &[
                        "device",
                        "modify",
                        &adapter.id,
                        "802-3-ethernet.mtu",
                        &mtu_str,
                    ],
                )
                .await?;
            }
            changed = true;
        }

        if let Some(dns) = &cfg.dns {
            if !dns.servers.is_empty() {
                let joined = dns.servers.join(" ");
                // Apply DNS globally to the primary device. Finding it:
                let primary = primary_device_nmcli().await?;
                if dry {
                    changed = true;
                } else {
                    run_checked(
                        "nmcli",
                        &["device", "modify", &primary, "ipv4.dns", &joined],
                    )
                    .await?;
                    changed = true;
                }
            }
        }

        Ok(if changed {
            if dry {
                SectionOutcome::WouldChange
            } else {
                SectionOutcome::Changed
            }
        } else {
            SectionOutcome::AlreadyCurrent
        })
    }

    async fn display(&self, cfg: &DisplayConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        if !command_exists("xrandr").await {
            return Err(ConfigError::NotSupported(
                "display reconciliation on linux requires xrandr (X11); wayland support is per-compositor and not yet implemented"
                    .into(),
            ));
        }
        if std::env::var("DISPLAY").is_err() {
            return Err(ConfigError::NotSupported(
                "DISPLAY is unset; the applier must run inside the user's X session".into(),
            ));
        }

        let mut changed = false;
        for monitor in &cfg.monitors {
            if dry {
                changed = true;
                continue;
            }
            apply_monitor_xrandr(monitor).await?;
            changed = true;
        }

        Ok(if changed {
            if dry {
                SectionOutcome::WouldChange
            } else {
                SectionOutcome::Changed
            }
        } else {
            SectionOutcome::AlreadyCurrent
        })
    }

    async fn desktop(
        &self,
        cfg: &BTreeMap<String, DesktopConfig>,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        if !command_exists("gsettings").await {
            return Err(ConfigError::NotSupported(
                "desktop reconciliation requires gsettings (GNOME / libgtk-based desktops); KDE support uses kwriteconfig5 and is not yet wired"
                    .into(),
            ));
        }
        let mut changed = false;
        for (user, desktop) in cfg {
            validate_username(user)?;
            for (key, value) in desktop_keys(desktop) {
                if dry {
                    changed = true;
                    continue;
                }
                run_gsettings_as(user, &key, &value).await?;
                changed = true;
            }
            for (raw_key, raw_value) in &desktop.raw {
                let (schema, key) = split_raw_key(raw_key)?;
                let value = json_value_to_gsettings(raw_value);
                if dry {
                    changed = true;
                    continue;
                }
                run_gsettings_set_schema(user, schema, key, &value).await?;
                changed = true;
            }
        }

        Ok(if changed {
            if dry {
                SectionOutcome::WouldChange
            } else {
                SectionOutcome::Changed
            }
        } else {
            SectionOutcome::AlreadyCurrent
        })
    }

    async fn shell(&self, cfg: &ShellConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        let mut changed = false;
        for (user, shell) in &cfg.default_shell {
            validate_username(user)?;
            if !Path::new(shell).exists() {
                return Err(ConfigError::InvalidSpec(format!(
                    "shell path does not exist: {shell}"
                )));
            }
            let current = current_shell_linux(user).await?;
            if current.as_deref() == Some(shell.as_str()) {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }
            run_checked("chsh", &["-s", shell, user]).await?;
            changed = true;
        }

        Ok(if changed {
            if dry {
                SectionOutcome::WouldChange
            } else {
                SectionOutcome::Changed
            }
        } else {
            SectionOutcome::AlreadyCurrent
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn validate_hostname(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.len() > 64 {
        return Err(ConfigError::InvalidSpec(
            "hostname must be 1-64 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(ConfigError::InvalidSpec(
            "hostname must contain only letters, digits, hyphens, and dots".into(),
        ));
    }
    Ok(())
}

fn validate_iana_tz(iana: &str) -> Result<(), ConfigError> {
    if iana.is_empty()
        || iana.contains("..")
        || iana.starts_with('/')
        || !iana
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '+')
    {
        return Err(ConfigError::InvalidSpec(format!(
            "invalid IANA timezone: {iana}"
        )));
    }
    Ok(())
}

fn validate_username(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.len() > 32 {
        return Err(ConfigError::InvalidSpec(
            "username must be 1-32 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(ConfigError::InvalidSpec(
            "username must contain only letters, digits, underscore, hyphen, and dot".into(),
        ));
    }
    if name.starts_with('-') {
        return Err(ConfigError::InvalidSpec(
            "username cannot start with a hyphen".into(),
        ));
    }
    Ok(())
}

async fn read_trimmed(path: &str) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|s| s.trim().to_string())
}

async fn run_checked(bin: &str, args: &[&str]) -> Result<Output, ConfigError> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                ConfigError::NotSupported(format!("{bin} not found on PATH"))
            }
            _ => ConfigError::Platform(format!("{bin}: {e}")),
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lower = stderr.to_lowercase();
        if lower.contains("permission denied") || lower.contains("must be root") {
            return Err(ConfigError::PermissionDenied(format!(
                "{bin}: {}",
                stderr.trim()
            )));
        }
        return Err(ConfigError::Platform(format!("{bin}: {}", stderr.trim())));
    }
    Ok(out)
}

async fn run_capture(bin: &str, args: &[&str]) -> Result<String, ConfigError> {
    let out = run_checked(bin, args).await?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn command_exists(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success() || !o.stderr.is_empty() || !o.stdout.is_empty())
        .unwrap_or(false)
}

async fn current_timezone_linux() -> Option<String> {
    // `timedatectl show --property=Timezone --value` is the stable interface.
    let out = Command::new("timedatectl")
        .args(["show", "--property=Timezone", "--value"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn user_exists_linux(name: &str) -> bool {
    Command::new("id")
        .arg(name)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn create_user_linux(user: &UserConfig) -> Result<(), ConfigError> {
    let mut args: Vec<String> = vec!["-m".into()];
    if let Some(full) = &user.full_name {
        args.push("-c".into());
        args.push(full.clone());
    }
    if !user.groups.is_empty() {
        args.push("-G".into());
        args.push(user.groups.join(","));
    }
    args.push(user.name.clone());
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("useradd", &args_ref).await?;

    if let Some(pw) = &user.initial_password {
        set_password_linux(&user.name, pw).await?;
        if user.must_change_password {
            run_checked("passwd", &["-e", &user.name]).await?;
        }
    }
    if user.admin {
        run_checked("usermod", &["-aG", "sudo", &user.name]).await?;
    }
    Ok(())
}

async fn reconcile_user_linux(user: &UserConfig, dry: bool) -> Result<bool, ConfigError> {
    // Compare groups and admin flag with the current state.
    let current_groups = current_groups_linux(&user.name).await?;
    let desired_groups: std::collections::BTreeSet<&str> =
        user.groups.iter().map(String::as_str).collect();
    let current_set: std::collections::BTreeSet<&str> =
        current_groups.iter().map(String::as_str).collect();

    let missing: Vec<&str> = desired_groups.difference(&current_set).copied().collect();
    let admin_current = current_groups.iter().any(|g| g == "sudo" || g == "wheel");
    let admin_mismatch = user.admin && !admin_current;

    if missing.is_empty() && !admin_mismatch {
        return Ok(false);
    }
    if dry {
        return Ok(true);
    }

    if !missing.is_empty() {
        let joined = missing.join(",");
        run_checked("usermod", &["-aG", &joined, &user.name]).await?;
    }
    if admin_mismatch {
        run_checked("usermod", &["-aG", "sudo", &user.name]).await?;
    }
    Ok(true)
}

async fn current_groups_linux(name: &str) -> Result<Vec<String>, ConfigError> {
    let out = run_capture("id", &["-nG", name]).await?;
    Ok(out.split_whitespace().map(|s| s.to_string()).collect())
}

async fn set_password_linux(name: &str, password: &str) -> Result<(), ConfigError> {
    // chpasswd reads "user:password\n" on stdin and is the canonical
    // non-interactive password setter.
    use tokio::io::AsyncWriteExt;
    let mut child = Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ConfigError::NotSupported("chpasswd not found".into()),
            _ => ConfigError::Platform(format!("chpasswd: {e}")),
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ConfigError::Platform("chpasswd did not give us a stdin pipe".into()))?;
    let line = format!("{name}:{password}\n");
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| ConfigError::Platform(format!("chpasswd stdin: {e}")))?;
    drop(stdin);
    let status = child
        .wait()
        .await
        .map_err(|e| ConfigError::Platform(format!("chpasswd wait: {e}")))?;
    if !status.success() {
        return Err(ConfigError::Platform("chpasswd returned non-zero".into()));
    }
    Ok(())
}

async fn primary_device_nmcli() -> Result<String, ConfigError> {
    let out = run_capture("nmcli", &["-t", "-f", "DEVICE,STATE", "device", "status"]).await?;
    for line in out.lines() {
        let mut parts = line.split(':');
        let device = parts.next().unwrap_or("");
        let state = parts.next().unwrap_or("");
        if state == "connected" && device != "lo" {
            return Ok(device.to_string());
        }
    }
    Err(ConfigError::Platform("no connected network device".into()))
}

fn parse_nmcli_ipv4(out: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(val) = line.strip_prefix("IP4.ADDRESS[1]:") {
            return Some(val.trim().to_string());
        }
    }
    None
}

async fn apply_monitor_xrandr(mon: &MonitorConfig) -> Result<(), ConfigError> {
    let mode = format!("{}x{}", mon.resolution.width, mon.resolution.height);
    let mut args: Vec<String> = vec!["--output".into(), mon.id.clone(), "--mode".into(), mode];
    if let Some(hz) = mon.refresh_hz {
        args.push("--rate".into());
        args.push(hz.to_string());
    }
    if let Some(pos) = mon.position {
        args.push("--pos".into());
        args.push(format!("{}x{}", pos.x, pos.y));
    }
    if mon.primary {
        args.push("--primary".into());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("xrandr", &refs).await?;
    Ok(())
}

async fn current_shell_linux(name: &str) -> Result<Option<String>, ConfigError> {
    let out = run_capture("getent", &["passwd", name]).await?;
    Ok(out
        .lines()
        .next()
        .and_then(|l| l.split(':').nth(6).map(String::from)))
}

fn desktop_keys(desktop: &DesktopConfig) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(wp) = &desktop.wallpaper {
        let uri = if wp.path.starts_with("file://") || wp.path.contains("://") {
            wp.path.clone()
        } else {
            format!("file://{}", wp.path)
        };
        out.push((
            "org.gnome.desktop.background picture-uri".into(),
            format!("'{}'", uri.replace('\'', "\\'")),
        ));
        if let Some(fit) = wp.fit {
            out.push((
                "org.gnome.desktop.background picture-options".into(),
                format!("'{}'", wallpaper_fit_gsettings(fit)),
            ));
        }
    }
    if let Some(theme) = &desktop.theme {
        if let Some(mode) = &theme.mode {
            let gtk_theme = match mode.as_str() {
                "dark" => "Adwaita-dark",
                "light" => "Adwaita",
                _ => "Adwaita",
            };
            out.push((
                "org.gnome.desktop.interface color-scheme".into(),
                format!(
                    "'{}'",
                    if mode == "dark" {
                        "prefer-dark"
                    } else {
                        "prefer-light"
                    }
                ),
            ));
            out.push((
                "org.gnome.desktop.interface gtk-theme".into(),
                format!("'{gtk_theme}'"),
            ));
        }
    }
    if let Some(tb) = &desktop.taskbar {
        if let Some(hide) = tb.auto_hide {
            out.push((
                "org.gnome.shell.extensions.dash-to-dock dock-fixed".into(),
                (!hide).to_string(),
            ));
        }
    }
    out
}

fn wallpaper_fit_gsettings(fit: WallpaperFit) -> &'static str {
    match fit {
        WallpaperFit::Center => "centered",
        WallpaperFit::Stretch => "stretched",
        WallpaperFit::Fill => "zoom",
        WallpaperFit::Fit => "scaled",
        WallpaperFit::Tile => "wallpaper",
        WallpaperFit::Span => "spanned",
    }
}

async fn run_gsettings_as(user: &str, schema_key: &str, value: &str) -> Result<(), ConfigError> {
    // gsettings writes to the dconf daemon, which is per-user. `sudo -u`
    // plus DBUS_SESSION_BUS_ADDRESS is how Ansible and Puppet drive this.
    // The target user must have a running dbus session (they must be
    // logged in); without that, gsettings silently no-ops or fails.
    let mut parts = schema_key.split_whitespace();
    let schema = parts
        .next()
        .ok_or_else(|| ConfigError::InvalidSpec("empty gsettings key".into()))?;
    let key = parts
        .next()
        .ok_or_else(|| ConfigError::InvalidSpec("missing gsettings key name".into()))?;
    run_gsettings_set_schema(user, schema, key, value).await
}

async fn run_gsettings_set_schema(
    user: &str,
    schema: &str,
    key: &str,
    value: &str,
) -> Result<(), ConfigError> {
    run_checked(
        "sudo",
        &["-u", user, "-i", "gsettings", "set", schema, key, value],
    )
    .await?;
    Ok(())
}

fn split_raw_key(raw: &str) -> Result<(&str, &str), ConfigError> {
    raw.split_once(':').ok_or_else(|| {
        ConfigError::InvalidSpec(format!("raw desktop key must be 'schema:key', got {raw:?}"))
    })
}

fn json_value_to_gsettings(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "\\'")),
        other => other.to_string(),
    }
}
