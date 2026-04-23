//! Auto-detect the system package manager.

use std::path::Path;

/// Detected package manager type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgManagerType {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Apk,
    Zypper,
    Emerge,
    Brew,
    Winget,
    Pkg, // FreeBSD
}

impl PkgManagerType {
    pub fn name(&self) -> &str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pacman => "pacman",
            Self::Apk => "apk",
            Self::Zypper => "zypper",
            Self::Emerge => "emerge",
            Self::Brew => "brew",
            Self::Winget => "winget",
            Self::Pkg => "pkg",
        }
    }
}

/// Detect the system package manager by checking for known binaries.
pub fn detect() -> Option<PkgManagerType> {
    if cfg!(windows) {
        // WingetManager handles both CLI-in-PATH and the registry
        // Uninstall fallback internally, so the only way to get "no
        // package manager" on Windows is to not be on Windows. Don't
        // gate on `which("winget")` here because a LocalSystem
        // service can't invoke the per-user Store-delivered winget
        // binary, and we'd skip the fallback entirely.
        return Some(PkgManagerType::Winget);
    }

    if cfg!(target_os = "macos") {
        if which("brew") {
            return Some(PkgManagerType::Brew);
        }
        return None;
    }

    if cfg!(target_os = "freebsd") {
        if which("pkg") {
            return Some(PkgManagerType::Pkg);
        }
        return None;
    }

    // Linux: check in priority order
    let checks = [
        (PkgManagerType::Apt, "apt-get"),
        (PkgManagerType::Dnf, "dnf"),
        (PkgManagerType::Yum, "yum"),
        (PkgManagerType::Pacman, "pacman"),
        (PkgManagerType::Apk, "apk"),
        (PkgManagerType::Zypper, "zypper"),
        (PkgManagerType::Emerge, "emerge"),
    ];

    for (pm_type, binary) in &checks {
        if which(binary) {
            return Some(*pm_type);
        }
    }

    None
}

/// Check if a binary exists in PATH.
fn which(name: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path.split(sep) {
            let candidate = Path::new(dir).join(name);
            if candidate.exists() {
                return true;
            }
            // On Windows, also check .exe
            if cfg!(windows) {
                let with_ext = Path::new(dir).join(format!("{name}.exe"));
                if with_ext.exists() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_something() {
        // On any system, we should detect at least one package manager
        // (or None on minimal containers). Just verify it doesn't panic.
        let result = detect();
        if let Some(pm) = result {
            assert!(!pm.name().is_empty());
        }
    }

    #[test]
    fn which_finds_common_binary() {
        // "echo" exists on every platform (as a shell builtin or binary)
        // On Windows, cmd.exe always exists
        if cfg!(windows) {
            assert!(which("cmd"));
        }
        // On Unix, /usr/bin/env is basically universal
        #[cfg(unix)]
        assert!(which("env"));
    }
}
