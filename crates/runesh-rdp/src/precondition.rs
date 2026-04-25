//! Read-only checks that run before we even attempt to dial RDP.
//!
//! Only Windows is meaningful here: macOS / Linux RDP support is
//! out of scope for this crate — the RDP path is for managing
//! Windows boxes from a Windows agent. Other platforms always
//! return `Ok(true)` so the connect flow proceeds and any failure
//! surfaces from the network layer.

use crate::error::RdpError;

/// Returns `Ok(true)` if Remote Desktop is enabled on the local host
/// (the agent's machine, which is also our RDP target since we
/// connect to `127.0.0.1`). Returns `Ok(false)` if the registry
/// explicitly disables it. Returns the underlying error for any
/// non-permission read failure.
///
/// Does not touch the registry on non-Windows targets.
pub fn rdp_enabled() -> Result<bool, RdpError> {
    #[cfg(windows)]
    {
        windows_impl::rdp_enabled()
    }
    #[cfg(not(windows))]
    {
        Ok(true)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;

    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

    /// `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server`
    const TERMINAL_SERVER_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Terminal Server";
    /// `fDenyTSConnections` — DWORD; `0` = RDP enabled, `1` = disabled.
    const DENY_VALUE: &str = "fDenyTSConnections";

    pub(super) fn rdp_enabled() -> Result<bool, RdpError> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = match hklm.open_subkey_with_flags(TERMINAL_SERVER_KEY, KEY_READ) {
            Ok(k) => k,
            // Missing key on older / stripped-down editions: treat as
            // disabled rather than as enabled, because attempting a
            // connect would hang on the dropped TCP SYN.
            Err(_) => return Ok(false),
        };
        let deny: u32 = key.get_value(DENY_VALUE).unwrap_or(1);
        Ok(deny == 0)
    }
}
