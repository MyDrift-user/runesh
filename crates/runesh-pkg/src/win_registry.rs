//! Read-only installed-package enumeration from the Windows Uninstall registry.
//!
//! Works from any security context, including `LocalSystem` services that
//! cannot launch `winget.exe` (which is a Store-delivered per-user app living
//! under `C:\Program Files\WindowsApps\...`). Use as a fallback for
//! [`crate::winget::WingetManager::list_installed`] when the `winget` CLI
//! isn't reachable.
//!
//! Enumerates three keys, matching the pattern `runesh-inventory` already uses:
//!   * `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*`
//!   * `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*`
//!   * `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*`
//!
//! A service running as `LocalSystem` reads its own `HKCU` hive (empty on a
//! fresh system account) rather than the interactive user's — operators who
//! need per-user packages should rely on the `HKLM` keys, which is where the
//! overwhelming majority of line-of-business software registers.

use crate::PackageInfo;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

/// Enumerate installed packages from the Windows Uninstall registry keys.
///
/// Returns one [`PackageInfo`] per registered product that exposes a
/// non-empty `DisplayName`. Duplicates across hives are preserved — callers
/// that care about uniqueness should deduplicate by `name`.
pub fn list_installed() -> Vec<PackageInfo> {
    let mut out = Vec::new();
    enumerate(RegKey::predef(HKEY_LOCAL_MACHINE), UNINSTALL, &mut out);
    enumerate(RegKey::predef(HKEY_LOCAL_MACHINE), UNINSTALL_WOW, &mut out);
    enumerate(RegKey::predef(HKEY_CURRENT_USER), UNINSTALL, &mut out);
    out
}

fn enumerate(root: RegKey, subkey: &str, out: &mut Vec<PackageInfo>) {
    let Ok(base) = root.open_subkey_with_flags(subkey, KEY_READ) else {
        return;
    };
    for sub in base.enum_keys().flatten() {
        let Ok(entry) = base.open_subkey_with_flags(&sub, KEY_READ) else {
            continue;
        };
        let name: String = entry.get_value("DisplayName").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        // `SystemComponent=1` marks rows the Add/Remove Programs UI hides
        // (Windows Update entries, MSI components, patches). Skip them so
        // operators see a list that matches what they'd see in the UI.
        if entry.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1 {
            continue;
        }
        let version: String = entry.get_value("DisplayVersion").unwrap_or_default();
        let publisher: String = entry.get_value("Publisher").unwrap_or_default();
        out.push(PackageInfo {
            name,
            version,
            description: if publisher.is_empty() {
                None
            } else {
                Some(publisher)
            },
            installed: true,
            update_available: None,
        });
    }
}
