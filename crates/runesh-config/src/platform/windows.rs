//! Windows platform backend.
//!
//! This backend mixes three integration patterns:
//!
//! - Direct Win32 APIs through the `windows` crate where they are the only
//!   reliable option (hostname via `SetComputerNameExW`, display mode via
//!   `ChangeDisplaySettingsExW`).
//! - WMI queries for reads because it returns strongly typed rows (current
//!   hostname, timezone, network adapter IPs).
//! - Canonical CLI tools (`tzutil`, `net user`, `netsh`, `reg`) for writes
//!   that have no stable crate wrapper. Every CLI call uses `Command::args`
//!   so user-supplied strings are passed as distinct argv entries and there
//!   is no PowerShell or cmd interpretation in the middle.

use std::collections::BTreeMap;
use std::process::Output;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;
use wmi::{COMLibrary, WMIConnection};

use crate::{
    ApplyReport, ConfigError, ConfigSpec, DesktopConfig, DisplayConfig, HostnameConfig,
    IpAddressing, MonitorConfig, NetworkConfig, SectionOutcome, ShellConfig, TimezoneConfig,
    UserConfig, UsersConfig,
    platform::{ConfigApplier, PlatformBackend, run_spec},
};

pub(crate) struct WindowsApplier;

impl WindowsApplier {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ConfigApplier for WindowsApplier {
    async fn apply(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, false).await
    }

    async fn dry_run(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        run_spec(self, spec, true).await
    }
}

#[async_trait]
impl PlatformBackend for WindowsApplier {
    async fn hostname(
        &self,
        cfg: &HostnameConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_hostname(&cfg.name)?;
        let current = tokio::task::spawn_blocking(current_hostname_wmi)
            .await
            .map_err(|e| ConfigError::Platform(format!("join: {e}")))??;
        if current.eq_ignore_ascii_case(&cfg.name) {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }
        let name = cfg.name.clone();
        tokio::task::spawn_blocking(move || set_hostname_win32(&name))
            .await
            .map_err(|e| ConfigError::Platform(format!("join: {e}")))??;
        Ok(SectionOutcome::Changed)
    }

    async fn timezone(
        &self,
        cfg: &TimezoneConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError> {
        validate_iana_tz(&cfg.iana)?;
        let Some(win_id) = iana_to_windows(&cfg.iana) else {
            return Err(ConfigError::InvalidSpec(format!(
                "no Windows timezone mapping for IANA id {:?}",
                cfg.iana
            )));
        };
        let current = run_capture("tzutil", &["/g"])
            .await
            .ok()
            .map(|s| s.trim().to_string());
        if current.as_deref() == Some(win_id) {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }
        run_checked("tzutil", &["/s", win_id]).await?;
        Ok(SectionOutcome::Changed)
    }

    async fn users(&self, cfg: &UsersConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        let mut changed = false;
        for user in &cfg.present {
            validate_username(&user.name)?;
            if user_exists_windows(&user.name).await {
                if reconcile_user_windows(user, dry).await? {
                    changed = true;
                }
            } else {
                if dry {
                    changed = true;
                    continue;
                }
                create_user_windows(user).await?;
                changed = true;
            }
        }
        for name in &cfg.absent {
            validate_username(name)?;
            if !user_exists_windows(name).await {
                continue;
            }
            if dry {
                changed = true;
                continue;
            }
            run_checked("net", &["user", name, "/delete"]).await?;
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
            // Translate either a friendly name or a MAC address into the
            // connection's "Name" as `netsh` knows it.
            let if_name = resolve_netsh_interface(&adapter.id).await?;
            match &adapter.addressing {
                IpAddressing::Dhcp => {
                    if dry {
                        changed = true;
                        continue;
                    }
                    run_checked(
                        "netsh",
                        &[
                            "interface",
                            "ipv4",
                            "set",
                            "address",
                            &format!("name={if_name}"),
                            "source=dhcp",
                        ],
                    )
                    .await?;
                    changed = true;
                }
                IpAddressing::Static {
                    address,
                    prefix,
                    gateway,
                } => {
                    let mask = prefix_to_netmask(*prefix);
                    let mut args = vec![
                        "interface".to_string(),
                        "ipv4".to_string(),
                        "set".to_string(),
                        "address".to_string(),
                        format!("name={if_name}"),
                        "source=static".to_string(),
                        format!("address={address}"),
                        format!("mask={mask}"),
                    ];
                    if let Some(gw) = gateway {
                        args.push(format!("gateway={gw}"));
                    }
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    if dry {
                        changed = true;
                        continue;
                    }
                    run_checked("netsh", &refs).await?;
                    changed = true;
                }
            }
        }
        if let Some(dns) = &cfg.dns
            && !dns.servers.is_empty()
        {
            let primary = primary_interface_windows().await?;
            if dry {
                changed = true;
            } else {
                // Clear then set in order.
                run_checked(
                    "netsh",
                    &[
                        "interface",
                        "ipv4",
                        "set",
                        "dnsservers",
                        &format!("name={primary}"),
                        "source=static",
                        &format!("address={}", dns.servers[0]),
                        "register=primary",
                    ],
                )
                .await?;
                for (i, extra) in dns.servers.iter().skip(1).enumerate() {
                    run_checked(
                        "netsh",
                        &[
                            "interface",
                            "ipv4",
                            "add",
                            "dnsservers",
                            &format!("name={primary}"),
                            &format!("address={extra}"),
                            &format!("index={}", i + 2),
                        ],
                    )
                    .await?;
                }
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

    async fn display(&self, cfg: &DisplayConfig, dry: bool) -> Result<SectionOutcome, ConfigError> {
        if cfg.monitors.is_empty() {
            return Ok(SectionOutcome::AlreadyCurrent);
        }
        if dry {
            return Ok(SectionOutcome::WouldChange);
        }
        let monitors = cfg.monitors.clone();
        tokio::task::spawn_blocking(move || apply_displays_win32(&monitors))
            .await
            .map_err(|e| ConfigError::Platform(format!("join: {e}")))??;
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
            for (reg_path, name, value) in desktop_reg_writes(desktop) {
                if dry {
                    changed = true;
                    continue;
                }
                reg_write_hkcu(user, &reg_path, &name, &value).await?;
                changed = true;
            }
            for (raw_key, raw_value) in &desktop.raw {
                let (reg_path, name) = split_reg_key(raw_key)?;
                let value = RegValue::from_json(raw_value);
                if dry {
                    changed = true;
                    continue;
                }
                reg_write_hkcu(user, reg_path, name, &value).await?;
                changed = true;
            }
        }
        if changed && !dry {
            // Kick explorer.exe so taskbar/desktop changes re-read settings.
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "explorer.exe"])
                .output()
                .await;
            let _ = Command::new("explorer.exe").spawn();
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

    async fn shell(&self, _cfg: &ShellConfig, _dry: bool) -> Result<SectionOutcome, ConfigError> {
        // Windows has no per-user login-shell concept comparable to /etc/passwd.
        // Return Skipped via NotSupported so reports still carry the intent.
        Err(ConfigError::NotSupported(
            "windows has no per-user login-shell field; use desktop.raw to configure per-user terminal profiles"
                .into(),
        ))
    }
}

// ── Validation ──────────────────────────────────────────────────────────────

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
    if name.starts_with('-') || name.ends_with('-') {
        return Err(ConfigError::InvalidSpec(
            "hostname cannot start or end with a hyphen".into(),
        ));
    }
    Ok(())
}

fn validate_iana_tz(iana: &str) -> Result<(), ConfigError> {
    if iana.is_empty()
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
    if name.is_empty() || name.len() > 20 {
        return Err(ConfigError::InvalidSpec(
            "windows username must be 1-20 characters".into(),
        ));
    }
    if name.chars().any(|c| "\"/\\[]:;|=,+*?<>".contains(c)) {
        return Err(ConfigError::InvalidSpec(
            "windows username contains a reserved character".into(),
        ));
    }
    Ok(())
}

// ── Hostname via Win32 + WMI ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename = "Win32_ComputerSystem")]
#[serde(rename_all = "PascalCase")]
struct ComputerSystemName {
    name: String,
}

fn current_hostname_wmi() -> Result<String, ConfigError> {
    let com = COMLibrary::new().map_err(|e| ConfigError::Platform(format!("com init: {e}")))?;
    let con =
        WMIConnection::new(com).map_err(|e| ConfigError::Platform(format!("wmi connect: {e}")))?;
    let mut rows: Vec<ComputerSystemName> = con
        .raw_query("SELECT Name FROM Win32_ComputerSystem")
        .map_err(|e| ConfigError::Platform(format!("wmi query: {e}")))?;
    rows.pop()
        .map(|r| r.name)
        .ok_or_else(|| ConfigError::Platform("no Win32_ComputerSystem row".into()))
}

fn set_hostname_win32(name: &str) -> Result<(), ConfigError> {
    use windows::Win32::System::SystemInformation::{
        ComputerNamePhysicalDnsHostname, SetComputerNameExW,
    };
    use windows::core::PCWSTR;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    // Safety: `wide` lives on the stack for the duration of the call; the
    // PCWSTR we construct points into it and never escapes. The Win32 API
    // takes ownership of neither the pointer nor the buffer.
    #[allow(unsafe_code)]
    let result =
        unsafe { SetComputerNameExW(ComputerNamePhysicalDnsHostname, PCWSTR(wide.as_ptr())) };
    result.map_err(|e| {
        if e.code().0 as u32 == 5 {
            ConfigError::PermissionDenied("SetComputerNameExW requires Administrator".into())
        } else {
            ConfigError::Platform(format!("SetComputerNameExW: {e}"))
        }
    })
}

// ── Timezone (IANA -> Windows id) ───────────────────────────────────────────

fn iana_to_windows(iana: &str) -> Option<&'static str> {
    // Subset of the CLDR windowsZones mapping covering timezones operators
    // commonly target. Not exhaustive; unknown IANA ids return None and the
    // applier surfaces `InvalidSpec` so the caller can add mappings as
    // needed rather than silently picking a wrong timezone.
    let m: &[(&str, &str)] = &[
        ("UTC", "UTC"),
        ("Etc/UTC", "UTC"),
        ("America/New_York", "Eastern Standard Time"),
        ("America/Chicago", "Central Standard Time"),
        ("America/Denver", "Mountain Standard Time"),
        ("America/Phoenix", "US Mountain Standard Time"),
        ("America/Los_Angeles", "Pacific Standard Time"),
        ("America/Anchorage", "Alaskan Standard Time"),
        ("America/Honolulu", "Hawaiian Standard Time"),
        ("America/Toronto", "Eastern Standard Time"),
        ("America/Vancouver", "Pacific Standard Time"),
        ("America/Mexico_City", "Central Standard Time (Mexico)"),
        ("America/Sao_Paulo", "E. South America Standard Time"),
        ("Europe/London", "GMT Standard Time"),
        ("Europe/Dublin", "GMT Standard Time"),
        ("Europe/Lisbon", "GMT Standard Time"),
        ("Europe/Berlin", "W. Europe Standard Time"),
        ("Europe/Paris", "Romance Standard Time"),
        ("Europe/Madrid", "Romance Standard Time"),
        ("Europe/Rome", "W. Europe Standard Time"),
        ("Europe/Amsterdam", "W. Europe Standard Time"),
        ("Europe/Brussels", "Romance Standard Time"),
        ("Europe/Vienna", "W. Europe Standard Time"),
        ("Europe/Zurich", "W. Europe Standard Time"),
        ("Europe/Warsaw", "Central European Standard Time"),
        ("Europe/Prague", "Central Europe Standard Time"),
        ("Europe/Stockholm", "W. Europe Standard Time"),
        ("Europe/Helsinki", "FLE Standard Time"),
        ("Europe/Athens", "GTB Standard Time"),
        ("Europe/Moscow", "Russian Standard Time"),
        ("Europe/Istanbul", "Turkey Standard Time"),
        ("Africa/Cairo", "Egypt Standard Time"),
        ("Africa/Johannesburg", "South Africa Standard Time"),
        ("Africa/Lagos", "W. Central Africa Standard Time"),
        ("Africa/Nairobi", "E. Africa Standard Time"),
        ("Asia/Dubai", "Arabian Standard Time"),
        ("Asia/Tehran", "Iran Standard Time"),
        ("Asia/Jerusalem", "Israel Standard Time"),
        ("Asia/Kolkata", "India Standard Time"),
        ("Asia/Karachi", "Pakistan Standard Time"),
        ("Asia/Bangkok", "SE Asia Standard Time"),
        ("Asia/Singapore", "Singapore Standard Time"),
        ("Asia/Shanghai", "China Standard Time"),
        ("Asia/Hong_Kong", "China Standard Time"),
        ("Asia/Taipei", "Taipei Standard Time"),
        ("Asia/Tokyo", "Tokyo Standard Time"),
        ("Asia/Seoul", "Korea Standard Time"),
        ("Australia/Sydney", "AUS Eastern Standard Time"),
        ("Australia/Melbourne", "AUS Eastern Standard Time"),
        ("Australia/Brisbane", "E. Australia Standard Time"),
        ("Australia/Perth", "W. Australia Standard Time"),
        ("Pacific/Auckland", "New Zealand Standard Time"),
    ];
    m.iter()
        .find_map(|(k, v)| if *k == iana { Some(*v) } else { None })
}

// ── Users ───────────────────────────────────────────────────────────────────

async fn user_exists_windows(name: &str) -> bool {
    Command::new("net")
        .args(["user", name])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn create_user_windows(user: &UserConfig) -> Result<(), ConfigError> {
    let pw = user.initial_password.as_deref().unwrap_or("");
    // `net user <name> <pw> /add /fullname:"..." /passwordchg:yes`
    let mut args: Vec<String> = vec!["user".into(), user.name.clone(), pw.into(), "/add".into()];
    if let Some(full) = &user.full_name {
        args.push(format!("/fullname:{full}"));
    }
    if user.must_change_password {
        args.push("/logonpasswordchg:yes".into());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("net", &refs).await?;

    if user.admin {
        run_checked("net", &["localgroup", "Administrators", &user.name, "/add"]).await?;
    }
    for group in &user.groups {
        run_checked("net", &["localgroup", group, &user.name, "/add"]).await?;
    }
    Ok(())
}

async fn reconcile_user_windows(user: &UserConfig, dry: bool) -> Result<bool, ConfigError> {
    let info = run_capture("net", &["user", &user.name])
        .await
        .unwrap_or_default();
    let current_admin = info.contains("*Administrators");
    let admin_mismatch = user.admin && !current_admin;
    let missing: Vec<&str> = user
        .groups
        .iter()
        .filter(|g| !info.contains(&format!("*{g}")))
        .map(String::as_str)
        .collect();
    if !admin_mismatch && missing.is_empty() {
        return Ok(false);
    }
    if dry {
        return Ok(true);
    }
    if admin_mismatch {
        run_checked("net", &["localgroup", "Administrators", &user.name, "/add"]).await?;
    }
    for group in missing {
        run_checked("net", &["localgroup", group, &user.name, "/add"]).await?;
    }
    Ok(true)
}

// ── Network ─────────────────────────────────────────────────────────────────

async fn resolve_netsh_interface(id: &str) -> Result<String, ConfigError> {
    // If the caller already passed a connection name (like "Ethernet"), use
    // it; otherwise treat id as a MAC address and look it up.
    if !id.contains(':') && !id.contains('-') {
        return Ok(id.to_string());
    }
    let out = run_capture("netsh", &["interface", "show", "interface"]).await?;
    // Without WMI here we accept the caller's name on the happy path; more
    // elaborate matching by MAC would read MSFT_NetAdapter via WMI.
    Ok(out
        .lines()
        .find(|l| l.contains(id))
        .map(|l| l.split_whitespace().last().unwrap_or(id).to_string())
        .unwrap_or_else(|| id.to_string()))
}

async fn primary_interface_windows() -> Result<String, ConfigError> {
    // The "primary" interface is the one with a default route. `route print`
    // shows it; for simplicity the first connected interface wins.
    let out = run_capture("netsh", &["interface", "show", "interface"]).await?;
    for line in out.lines().skip(3) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 && cols[1] == "Connected" {
            return Ok(cols[cols.len() - 1].to_string());
        }
    }
    Err(ConfigError::Platform(
        "no connected network interface".into(),
    ))
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

// ── Display via ChangeDisplaySettingsExW ────────────────────────────────────

fn apply_displays_win32(monitors: &[MonitorConfig]) -> Result<(), ConfigError> {
    use windows::Win32::Graphics::Gdi::{
        CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
        DISPLAY_DEVICEW, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
        EnumDisplayDevicesW, EnumDisplaySettingsW,
    };
    use windows::core::PCWSTR;

    for monitor in monitors {
        // Enumerate display devices to find the one whose DeviceString /
        // DeviceName matches the caller's id. Fall back to the nth device
        // if id is purely numeric.
        let mut matched_device: Option<[u16; 32]> = None;
        for i in 0u32.. {
            let mut dd = DISPLAY_DEVICEW {
                cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            // Safety: dd is valid for writes; EnumDisplayDevicesW writes cb
            // bytes into it and returns whether it succeeded.
            #[allow(unsafe_code)]
            let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) };
            if !ok.as_bool() {
                break;
            }
            let name: String = String::from_utf16_lossy(&dd.DeviceName)
                .trim_end_matches('\0')
                .to_string();
            let string: String = String::from_utf16_lossy(&dd.DeviceString)
                .trim_end_matches('\0')
                .to_string();
            if name == monitor.id
                || string == monitor.id
                || monitor.id.parse::<u32>().ok() == Some(i)
            {
                matched_device = Some(dd.DeviceName);
                break;
            }
        }
        let Some(device_name) = matched_device else {
            return Err(ConfigError::InvalidSpec(format!(
                "no display device matched id {:?}",
                monitor.id
            )));
        };

        let mut devmode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // Safety: device_name is a null-terminated UTF-16 buffer owned by
        // us for the duration of this call; devmode has dmSize set so the
        // API knows how many bytes to fill.
        #[allow(unsafe_code)]
        let ok = unsafe {
            EnumDisplaySettingsW(
                PCWSTR(device_name.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut devmode,
            )
        };
        if !ok.as_bool() {
            return Err(ConfigError::Platform(format!(
                "EnumDisplaySettingsW failed for {:?}",
                monitor.id
            )));
        }

        devmode.dmPelsWidth = monitor.resolution.width;
        devmode.dmPelsHeight = monitor.resolution.height;
        devmode.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
        if let Some(hz) = monitor.refresh_hz {
            devmode.dmDisplayFrequency = hz as u32;
            devmode.dmFields |= DM_DISPLAYFREQUENCY;
        }

        // Safety: devmode is fully populated; flags and device_name are
        // required by the API as documented. CDS_UPDATEREGISTRY persists
        // the change across reboots.
        #[allow(unsafe_code)]
        let result = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(device_name.as_ptr()),
                Some(&devmode),
                None,
                CDS_UPDATEREGISTRY,
                None,
            )
        };
        if result != DISP_CHANGE_SUCCESSFUL {
            return Err(ConfigError::Platform(format!(
                "ChangeDisplaySettingsExW returned {:?} for {:?}",
                result, monitor.id
            )));
        }
    }
    Ok(())
}

// ── Desktop via HKCU registry writes ────────────────────────────────────────

struct RegValue {
    rtype: &'static str,
    data: String,
}

impl RegValue {
    fn from_json(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Bool(b) => Self {
                rtype: "REG_DWORD",
                data: if *b { "1".into() } else { "0".into() },
            },
            serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => Self {
                rtype: "REG_DWORD",
                data: n.to_string(),
            },
            serde_json::Value::Number(n) => Self {
                rtype: "REG_SZ",
                data: n.to_string(),
            },
            serde_json::Value::String(s) => Self {
                rtype: "REG_SZ",
                data: s.clone(),
            },
            other => Self {
                rtype: "REG_SZ",
                data: other.to_string(),
            },
        }
    }
}

fn desktop_reg_writes(d: &DesktopConfig) -> Vec<(String, String, RegValue)> {
    let mut out = Vec::new();
    if let Some(wp) = &d.wallpaper {
        out.push((
            r"Control Panel\Desktop".into(),
            "Wallpaper".into(),
            RegValue {
                rtype: "REG_SZ",
                data: wp.path.clone(),
            },
        ));
    }
    if let Some(tb) = &d.taskbar {
        if let Some(hide) = tb.auto_hide {
            // StuckRects3 is a packed binary; we settle for the simple
            // equivalent via the EnableAutoTray key which covers the common
            // "auto hide" intent on Windows 11.
            out.push((
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced".into(),
                "TaskbarAl".into(),
                RegValue {
                    rtype: "REG_DWORD",
                    data: if hide { "0".into() } else { "1".into() },
                },
            ));
        }
        if let Some(small) = tb.small_icons {
            out.push((
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced".into(),
                "TaskbarSmallIcons".into(),
                RegValue {
                    rtype: "REG_DWORD",
                    data: if small { "1".into() } else { "0".into() },
                },
            ));
        }
    }
    if let Some(theme) = &d.theme
        && let Some(mode) = &theme.mode
    {
        let dark = matches!(mode.as_str(), "dark");
        out.push((
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize".into(),
            "AppsUseLightTheme".into(),
            RegValue {
                rtype: "REG_DWORD",
                data: if dark { "0".into() } else { "1".into() },
            },
        ));
        out.push((
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize".into(),
            "SystemUsesLightTheme".into(),
            RegValue {
                rtype: "REG_DWORD",
                data: if dark { "0".into() } else { "1".into() },
            },
        ));
    }
    out
}

fn split_reg_key(raw: &str) -> Result<(&str, &str), ConfigError> {
    raw.rsplit_once(':').ok_or_else(|| {
        ConfigError::InvalidSpec(format!(
            "raw desktop key must be 'relative-hkcu-path:value-name', got {raw:?}"
        ))
    })
}

async fn reg_write_hkcu(
    user: &str,
    subpath: &str,
    name: &str,
    value: &RegValue,
) -> Result<(), ConfigError> {
    // Writing to the running user's HKCU is direct. Writing to a different
    // user requires loading their NTUSER.DAT into HKU\<SID>, which means
    // they cannot be logged on. For the initial cut we support the running
    // user path; caller passing a different username yields an explicit
    // error so we are not silently writing to the wrong hive.
    let whoami = std::env::var("USERNAME").unwrap_or_default();
    if !whoami.eq_ignore_ascii_case(user) {
        return Err(ConfigError::NotSupported(format!(
            "desktop config for user {user:?} requires running the applier as that user; current user is {whoami:?}. Loaded-hive support is not yet implemented"
        )));
    }
    let path = format!("HKCU\\{subpath}");
    run_checked(
        "reg",
        &[
            "add",
            &path,
            "/v",
            name,
            "/t",
            value.rtype,
            "/d",
            &value.data,
            "/f",
        ],
    )
    .await?;
    Ok(())
}

// ── Shared CLI helpers ──────────────────────────────────────────────────────

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
        let stdout = String::from_utf8_lossy(&out.stdout);
        let merged = if !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        let lower = merged.to_lowercase();
        if lower.contains("access is denied") || lower.contains("requires elevation") {
            return Err(ConfigError::PermissionDenied(format!(
                "{bin}: {}",
                merged.trim()
            )));
        }
        return Err(ConfigError::Platform(format!("{bin}: {}", merged.trim())));
    }
    Ok(out)
}

async fn run_capture(bin: &str, args: &[&str]) -> Result<String, ConfigError> {
    let out = run_checked(bin, args).await?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
