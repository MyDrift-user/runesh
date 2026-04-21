//! macOS platform backend.
//!
//! Uses the native command-line tools that ship in `/usr/bin` and
//! `/usr/sbin` on every macOS install: `scutil`, `systemsetup`, `dscl`,
//! `sysadminctl`, `networksetup`, `defaults`, `chsh`, `displayplacer`.
//! All are invoked via `Command::args` (never a shell). Operations that
//! require administrator privilege return
//! [`ConfigError::PermissionDenied`] when the applier is not running as
//! root; callers are expected to run the agent under `launchd` at the
//! system level.

use std::collections::BTreeMap;
use std::process::Output;

use async_trait::async_trait;
use tokio::process::Command;

use crate::{
    ApplyReport, ConfigError, ConfigSpec, DesktopConfig, DisplayConfig, HostnameConfig,
    IpAddressing, MonitorConfig, NetworkConfig, SectionOutcome, ShellConfig, TimezoneConfig,
    UserConfig, UsersConfig,
    platform::{ConfigApplier, PlatformBackend, run_spec},
};

pub(crate) struct MacOsApplier;

impl MacOsApplier {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ConfigApplier for MacOsApplier {
    async fn apply(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, false).await
    }

    async fn dry_run(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, true).await
    }
}

#[async_trait]
impl PlatformBackend for MacOsApplier {
    async fn hostname(
        &self,
        cfg: &HostnameConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_hostname(&cfg.name)?;
        let current = run_capture("scutil", &["--get", "HostName"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if current == cfg.name {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }

        // scutil --set updates one name at a time; setting HostName alone
        // leaves LocalHostName and ComputerName out of sync. Real macOS MDM
        // profiles set all three to the same value; do the same here.
        for key in ["HostName", "LocalHostName", "ComputerName"] {
            run_checked("scutil", &["--set", key, &cfg.name]).await?;
        }
        Ok(SectionOutcome::Changed)
    }

    async fn timezone(
        &self,
        cfg: &TimezoneConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_iana_tz(&cfg.iana)?;
        let current = run_capture("systemsetup", &["-gettimezone"])
            .await
            .ok()
            .and_then(|s| s.trim().strip_prefix("Time Zone: ").map(|v| v.to_string()));
        if current.as_deref() == Some(cfg.iana.as_str()) {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }
        run_checked("systemsetup", &["-settimezone", &cfg.iana]).await?;
        Ok(SectionOutcome::Changed)
    }

    async fn users(&self, cfg: &UsersConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        let mut changed = false;

        for user in &cfg.present {
            validate_username(&user.name)?;
            if user_exists_macos(&user.name).await {
                if reconcile_user_macos(user, dry).await? {
                    changed = true;
                }
            } else {
                if dry {
                    changed = true;
                    continue;
                }
                create_user_macos(user).await?;
                changed = true;
            }
        }

        for name in &cfg.absent {
            validate_username(name)?;
            if !user_exists_macos(name).await {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }
            run_checked("sysadminctl", &["-deleteUser", name]).await?;
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
        let mut changed = false;

        for adapter in &cfg.adapters {
            let service = service_name_for_adapter(&adapter.id).await?;
            match &adapter.addressing {
                IpAddressing::Dhcp => {
                    if dry {
                        changed = true;
                        continue;
                    }
                    run_checked("networksetup", &["-setdhcp", &service]).await?;
                    changed = true;
                }
                IpAddressing::Static {
                    address,
                    prefix,
                    gateway,
                } => {
                    let mask = prefix_to_netmask(*prefix);
                    let gw = gateway.clone().unwrap_or_default();
                    let current = run_capture("networksetup", &["-getinfo", &service])
                        .await
                        .unwrap_or_default();
                    if networksetup_matches(&current, address, &mask, gateway.as_deref()) {
                        continue;
                    }
                    if dry {
                        changed = true;
                        continue;
                    }
                    if gw.is_empty() {
                        run_checked(
                            "networksetup",
                            &["-setmanual", &service, address, &mask, "0.0.0.0"],
                        )
                        .await?;
                    } else {
                        run_checked(
                            "networksetup",
                            &["-setmanual", &service, address, &mask, &gw],
                        )
                        .await?;
                    }
                    changed = true;
                }
            }
        }

        if let Some(dns) = &cfg.dns {
            if !dns.servers.is_empty() {
                let service = primary_service_macos().await?;
                if dry {
                    changed = true;
                } else {
                    let mut args: Vec<&str> = vec!["-setdnsservers", &service];
                    for s in &dns.servers {
                        args.push(s);
                    }
                    run_checked("networksetup", &args).await?;
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
        // macOS display management through Core Graphics requires unsafe
        // FFI that is painful to get right. `displayplacer` is an open-
        // source CLI (Homebrew: `brew install displayplacer`) that wraps
        // CGConfigureDisplay* and is the de-facto tool. If it is not
        // installed we surface a clear message rather than leave the
        // applier silent.
        if !command_exists("displayplacer").await {
            return Err(ConfigError::NotSupported(
                "macOS display reconciliation requires `displayplacer` (install via Homebrew: brew install displayplacer)"
                    .into(),
            ));
        }
        if cfg.monitors.is_empty() {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }
        let mut args: Vec<String> = Vec::new();
        for m in &cfg.monitors {
            args.push(displayplacer_arg(m));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_checked("displayplacer", &refs).await?;
        Ok(SectionOutcome::Changed)
    }

    async fn desktop(
        &self,
        cfg: &BTreeMap<String, DesktopConfig>,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        let mut changed = false;
        for (user, desktop) in cfg {
            validate_username(user)?;
            for (domain, key, value) in desktop_defaults_writes(desktop) {
                if dry {
                    changed = true;
                    continue;
                }
                run_checked(
                    "sudo",
                    &[
                        "-u",
                        user,
                        "defaults",
                        "write",
                        &domain,
                        &key,
                        &value.flag(),
                        &value.value(),
                    ],
                )
                .await?;
                changed = true;
            }
            // defaults changes to com.apple.dock need `killall Dock` for live reload.
            if desktop.taskbar.is_some() && !dry {
                let _ = Command::new("sudo")
                    .args(["-u", user, "killall", "Dock"])
                    .output()
                    .await;
            }
            for (raw_key, raw_value) in &desktop.raw {
                let (domain, key) = split_defaults_key(raw_key)?;
                let payload = DefaultsPayload::from_json(raw_value);
                if dry {
                    changed = true;
                    continue;
                }
                run_checked(
                    "sudo",
                    &[
                        "-u",
                        user,
                        "defaults",
                        "write",
                        domain,
                        key,
                        &payload.flag(),
                        &payload.value(),
                    ],
                )
                .await?;
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
            if !std::path::Path::new(shell).exists() {
                return Err(ConfigError::InvalidSpec(format!(
                    "shell path does not exist: {shell}"
                )));
            }
            let current = run_capture(
                "dscl",
                &[".", "-read", &format!("/Users/{user}"), "UserShell"],
            )
            .await
            .unwrap_or_default();
            let current_shell = current
                .lines()
                .find(|l| l.starts_with("UserShell:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .map(String::from);
            if current_shell.as_deref() == Some(shell.as_str()) {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }
            run_checked(
                "dscl",
                &[
                    ".",
                    "-change",
                    &format!("/Users/{user}"),
                    "UserShell",
                    current_shell.as_deref().unwrap_or("/bin/zsh"),
                    shell,
                ],
            )
            .await?;
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

// ── Shared helpers ──────────────────────────────────────────────────────────

fn validate_hostname(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.len() > 63 {
        return Err(ConfigError::InvalidSpec(
            "hostname must be 1-63 characters".into(),
        ));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(ConfigError::InvalidSpec(
            "hostname must contain only letters, digits, and hyphens".into(),
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
    Ok(())
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
        if lower.contains("permission denied") || lower.contains("you must be root") {
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
    Command::new("which")
        .arg(bin)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn user_exists_macos(name: &str) -> bool {
    Command::new("dscl")
        .args([".", "-read", &format!("/Users/{name}")])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn create_user_macos(user: &UserConfig) -> Result<(), ConfigError> {
    let mut args: Vec<String> = vec!["-addUser".into(), user.name.clone()];
    if let Some(full) = &user.full_name {
        args.push("-fullName".into());
        args.push(full.clone());
    }
    if let Some(pw) = &user.initial_password {
        args.push("-password".into());
        args.push(pw.clone());
    }
    if user.admin {
        args.push("-admin".into());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("sysadminctl", &refs).await?;

    // sysadminctl does not support groups; apply via dseditgroup.
    for group in &user.groups {
        let _ = run_checked(
            "dseditgroup",
            &["-o", "edit", "-a", &user.name, "-t", "user", group],
        )
        .await;
    }
    Ok(())
}

async fn reconcile_user_macos(user: &UserConfig, dry: bool) -> Result<bool, ConfigError> {
    let mut changed = false;
    // Check admin membership via dsmemberutil.
    let admin_member = run_capture(
        "dsmemberutil",
        &["checkmembership", "-U", &user.name, "-G", "admin"],
    )
    .await
    .ok()
    .map(|s| s.contains("is a member"))
    .unwrap_or(false);
    if user.admin && !admin_member {
        if dry {
            return Ok(true);
        }
        run_checked(
            "dseditgroup",
            &["-o", "edit", "-a", &user.name, "-t", "user", "admin"],
        )
        .await?;
        changed = true;
    }
    for group in &user.groups {
        let member = run_capture(
            "dsmemberutil",
            &["checkmembership", "-U", &user.name, "-G", group],
        )
        .await
        .ok()
        .map(|s| s.contains("is a member"))
        .unwrap_or(false);
        if member {
            continue;
        }
        if dry {
            return Ok(true);
        }
        run_checked(
            "dseditgroup",
            &["-o", "edit", "-a", &user.name, "-t", "user", group],
        )
        .await?;
        changed = true;
    }
    Ok(changed)
}

async fn service_name_for_adapter(id: &str) -> Result<String, ConfigError> {
    // `networksetup -listallhardwareports` returns a human-readable mapping
    // between Hardware Port (the "service" name) and device id (like en0).
    let out = run_capture("networksetup", &["-listallhardwareports"]).await?;
    let mut current_port: Option<String> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port: ") {
            current_port = Some(p.trim().to_string());
        } else if let Some(dev) = line.strip_prefix("Device: ") {
            if dev.trim() == id {
                return current_port
                    .ok_or_else(|| ConfigError::Platform("no hardware port".into()));
            }
        }
    }
    // Maybe the caller passed the service name directly.
    Ok(id.to_string())
}

async fn primary_service_macos() -> Result<String, ConfigError> {
    let out = run_capture("networksetup", &["-listnetworkserviceorder"]).await?;
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("(1) ") {
            return Ok(name.trim().to_string());
        }
    }
    Err(ConfigError::Platform("no primary network service".into()))
}

fn prefix_to_netmask(prefix: u8) -> String {
    let bits: u32 = if prefix >= 32 {
        !0
    } else {
        !((1u32 << (32 - prefix)) - 1)
    };
    format!(
        "{}.{}.{}.{}",
        (bits >> 24) & 0xff,
        (bits >> 16) & 0xff,
        (bits >> 8) & 0xff,
        bits & 0xff
    )
}

fn networksetup_matches(info: &str, address: &str, netmask: &str, gateway: Option<&str>) -> bool {
    let mut ip_ok = false;
    let mut mask_ok = false;
    let mut gw_ok = gateway.is_none();
    for line in info.lines() {
        if let Some(v) = line.strip_prefix("IP address: ") {
            ip_ok = v.trim() == address;
        } else if let Some(v) = line.strip_prefix("Subnet mask: ") {
            mask_ok = v.trim() == netmask;
        } else if let Some(v) = line.strip_prefix("Router: ") {
            if let Some(gw) = gateway {
                gw_ok = v.trim() == gw;
            }
        }
    }
    ip_ok && mask_ok && gw_ok
}

fn displayplacer_arg(mon: &MonitorConfig) -> String {
    let mut s = format!(
        "id:{} res:{}x{}",
        mon.id, mon.resolution.width, mon.resolution.height
    );
    if let Some(hz) = mon.refresh_hz {
        s.push_str(&format!(" hz:{hz}"));
    }
    if let Some(scale) = mon.scale_percent {
        // displayplacer expects scaling:on (HiDPI) vs scaling:off.
        if scale > 100 {
            s.push_str(" scaling:on");
        } else {
            s.push_str(" scaling:off");
        }
    }
    if let Some(pos) = mon.position {
        s.push_str(&format!(" origin:({},{})", pos.x, pos.y));
    }
    if mon.primary {
        s.push_str(" degree:0");
    }
    s
}

// ── defaults write encoding ─────────────────────────────────────────────────

struct DefaultsPayload {
    flag: &'static str,
    value: String,
}

impl DefaultsPayload {
    fn from_json(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Bool(b) => Self {
                flag: "-bool",
                value: if *b { "true".into() } else { "false".into() },
            },
            serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => Self {
                flag: "-int",
                value: n.to_string(),
            },
            serde_json::Value::Number(n) => Self {
                flag: "-float",
                value: n.to_string(),
            },
            serde_json::Value::String(s) => Self {
                flag: "-string",
                value: s.clone(),
            },
            other => Self {
                flag: "-string",
                value: other.to_string(),
            },
        }
    }
    fn flag(&self) -> String {
        self.flag.to_string()
    }
    fn value(&self) -> String {
        self.value.clone()
    }
}

fn desktop_defaults_writes(d: &DesktopConfig) -> Vec<(String, String, DefaultsPayload)> {
    let mut out = Vec::new();
    if let Some(wp) = &d.wallpaper {
        // Wallpaper on macOS: use `osascript` to tell System Events; but
        // `defaults write com.apple.desktop Background` also works for the
        // simple case when a user is logged in. We use osascript externally,
        // not here, so expose this via the `raw` map for advanced cases.
        out.push((
            "com.apple.desktop".into(),
            "Background".into(),
            DefaultsPayload {
                flag: "-string",
                value: wp.path.clone(),
            },
        ));
    }
    if let Some(tb) = &d.taskbar {
        if let Some(hide) = tb.auto_hide {
            out.push((
                "com.apple.dock".into(),
                "autohide".into(),
                DefaultsPayload {
                    flag: "-bool",
                    value: if hide { "true".into() } else { "false".into() },
                },
            ));
        }
        if let Some(pos) = tb.position {
            let orientation = match pos {
                crate::TaskbarPosition::Bottom => "bottom",
                crate::TaskbarPosition::Left => "left",
                crate::TaskbarPosition::Right => "right",
                crate::TaskbarPosition::Top => "bottom", // macOS dock has no top
            };
            out.push((
                "com.apple.dock".into(),
                "orientation".into(),
                DefaultsPayload {
                    flag: "-string",
                    value: orientation.into(),
                },
            ));
        }
    }
    if let Some(theme) = &d.theme {
        if let Some(mode) = &theme.mode {
            let dark = matches!(mode.as_str(), "dark");
            out.push((
                "NSGlobalDomain".into(),
                "AppleInterfaceStyle".into(),
                DefaultsPayload {
                    flag: "-string",
                    value: if dark { "Dark".into() } else { "".into() },
                },
            ));
        }
    }
    out
}

fn split_defaults_key(raw: &str) -> Result<(&str, &str), ConfigError> {
    raw.split_once(':').ok_or_else(|| {
        ConfigError::InvalidSpec(format!("raw desktop key must be 'domain:key', got {raw:?}"))
    })
}
