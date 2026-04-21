//! Desired-state spec for an agent.
//!
//! Every field is optional. A caller ships only the sections they want to
//! enforce; missing sections are left untouched on the agent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level desired-state document.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigSpec {
    pub hostname: Option<HostnameConfig>,
    pub timezone: Option<TimezoneConfig>,
    pub users: Option<UsersConfig>,
    pub network: Option<NetworkConfig>,
    pub display: Option<DisplayConfig>,
    /// Per-user desktop configuration, keyed by username. Applied against
    /// HKCU on Windows, dconf/gsettings on GNOME, `defaults` on macOS.
    pub desktop: Option<BTreeMap<String, DesktopConfig>>,
    pub shell: Option<ShellConfig>,
}

// ── Hostname ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostnameConfig {
    /// Computer name. Must be 1-63 octets, LDH-compliant. The exact rules
    /// differ per OS (Windows tolerates up to 15 NetBIOS chars for the
    /// short name) but the validator accepts the intersection so the spec
    /// is portable.
    pub name: String,
}

// ── Timezone ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimezoneConfig {
    /// IANA timezone name, e.g. `"Europe/Zurich"`. Platform appliers translate
    /// to the native format (Windows uses "W. Europe Standard Time"; the
    /// applier handles the mapping).
    pub iana: String,
}

// ── Users ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsersConfig {
    /// Accounts that must exist.
    #[serde(default)]
    pub present: Vec<UserConfig>,
    /// Accounts that must not exist (best-effort deletion).
    #[serde(default)]
    pub absent: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserConfig {
    pub name: String,
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// True if the account is an administrator / sudoer.
    #[serde(default)]
    pub admin: bool,
    /// Plaintext password for initial provisioning only. The applier is
    /// expected to pass this to the OS user-creation API and not store it.
    #[serde(default)]
    pub initial_password: Option<String>,
    /// If true, require password change at next login.
    #[serde(default)]
    pub must_change_password: bool,
}

// ── Network ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkConfig {
    #[serde(default)]
    pub adapters: Vec<NetworkAdapterConfig>,
    #[serde(default)]
    pub dns: Option<DnsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkAdapterConfig {
    /// Interface name or MAC address as stable identifier.
    pub id: String,
    pub addressing: IpAddressing,
    #[serde(default)]
    pub mtu: Option<u32>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IpAddressing {
    Dhcp,
    Static {
        address: String,
        prefix: u8,
        gateway: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsConfig {
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default)]
    pub search: Vec<String>,
}

// ── Display ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayConfig {
    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorConfig {
    /// Stable identifier: EDID-based serial when available, index otherwise.
    pub id: String,
    pub resolution: Resolution,
    #[serde(default)]
    pub scale_percent: Option<u16>,
    #[serde(default)]
    pub refresh_hz: Option<u16>,
    #[serde(default)]
    pub position: Option<Position>,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

// ── Per-user desktop (taskbar, wallpaper, themes, dock) ─────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopConfig {
    #[serde(default)]
    pub wallpaper: Option<WallpaperConfig>,
    #[serde(default)]
    pub taskbar: Option<TaskbarConfig>,
    #[serde(default)]
    pub theme: Option<ThemeConfig>,
    /// Additional raw settings, keyed by backend namespace.
    /// Examples: `"org.gnome.desktop.interface:enable-animations" = "false"`,
    /// `"HKCU\\Control Panel\\Desktop:WallpaperStyle" = "10"`,
    /// `"com.apple.dock:autohide" = "true"`.
    #[serde(default)]
    pub raw: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WallpaperConfig {
    pub path: String,
    #[serde(default)]
    pub fit: Option<WallpaperFit>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperFit {
    Center,
    Stretch,
    Fill,
    Fit,
    Tile,
    Span,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskbarConfig {
    /// Keep taskbar visible always (false = auto-hide).
    #[serde(default)]
    pub always_on_top: Option<bool>,
    #[serde(default)]
    pub auto_hide: Option<bool>,
    #[serde(default)]
    pub position: Option<TaskbarPosition>,
    /// Small-icon mode on Windows; equivalent on macOS / Linux where available.
    #[serde(default)]
    pub small_icons: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskbarPosition {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeConfig {
    /// "light", "dark", "auto".
    pub mode: Option<String>,
    pub accent_color: Option<String>,
}

// ── Shell ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellConfig {
    /// Default login shell path per user (user -> shell path).
    #[serde(default)]
    pub default_shell: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_round_trips_empty() {
        let s = ConfigSpec::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: ConfigSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn spec_round_trips_full() {
        let mut desktop = BTreeMap::new();
        desktop.insert(
            "alice".into(),
            DesktopConfig {
                wallpaper: Some(WallpaperConfig {
                    path: "/home/alice/wall.png".into(),
                    fit: Some(WallpaperFit::Fill),
                }),
                taskbar: Some(TaskbarConfig {
                    auto_hide: Some(true),
                    position: Some(TaskbarPosition::Bottom),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let s = ConfigSpec {
            hostname: Some(HostnameConfig {
                name: "edge-01".into(),
            }),
            timezone: Some(TimezoneConfig {
                iana: "Europe/Zurich".into(),
            }),
            users: Some(UsersConfig {
                present: vec![UserConfig {
                    name: "alice".into(),
                    full_name: Some("Alice Admin".into()),
                    groups: vec!["docker".into()],
                    admin: true,
                    initial_password: None,
                    must_change_password: true,
                }],
                absent: vec!["legacy".into()],
            }),
            network: Some(NetworkConfig {
                adapters: vec![NetworkAdapterConfig {
                    id: "eth0".into(),
                    addressing: IpAddressing::Static {
                        address: "10.0.0.5".into(),
                        prefix: 24,
                        gateway: Some("10.0.0.1".into()),
                    },
                    mtu: Some(1500),
                    enabled: true,
                }],
                dns: Some(DnsConfig {
                    servers: vec!["1.1.1.1".into(), "9.9.9.9".into()],
                    search: vec!["corp.internal".into()],
                }),
            }),
            display: Some(DisplayConfig {
                monitors: vec![MonitorConfig {
                    id: "DP-1".into(),
                    resolution: Resolution {
                        width: 2560,
                        height: 1440,
                    },
                    scale_percent: Some(100),
                    refresh_hz: Some(60),
                    position: None,
                    primary: true,
                }],
            }),
            desktop: Some(desktop),
            shell: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ConfigSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
