//! UAC elevation helpers for Windows.
//!
//! Runs commands with Administrator privileges using the "runas" verb, which
//! triggers the Windows UAC prompt.
//!
//! Security:
//! - The binary path must be **absolute** and **exist as a regular file**.
//! - By default, the binary must live inside the same directory as the
//!   currently running executable (`std::env::current_exe()?.parent()?`).
//! - Callers may widen this with `run_elevated_in` and a caller-supplied
//!   allowlisted directory.
//!
//! Note: `ShellExecuteW` with `runas` launches the process asynchronously and
//! does not return a real child handle. The returned `ElevatedLaunch` carries
//! the launch return value so callers can observe success or failure of the
//! launch itself; it does not wait for the elevated process to exit.

use std::path::{Path, PathBuf};

/// Outcome of an elevation launch.
#[derive(Debug)]
pub struct ElevatedLaunch {
    /// The resolved, canonical binary path that was launched.
    pub binary: PathBuf,
}

/// Error returned by the elevation helpers.
#[derive(Debug, thiserror::Error)]
pub enum ElevationError {
    #[error("binary path must be absolute: {0}")]
    NotAbsolute(PathBuf),
    #[error("binary not found or not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("binary {binary} not in allowed directory {allowed}")]
    NotAllowed { binary: PathBuf, allowed: PathBuf },
    #[error("cannot resolve current exe directory: {0}")]
    CurrentExe(String),
    #[error("canonicalize failed for {0}: {1}")]
    Canonicalize(PathBuf, String),
    #[error("ShellExecuteW failed (code {0})")]
    ShellExec(usize),
}

/// Run a binary elevated, restricted to the same directory as the running
/// executable.
pub fn run_elevated(binary: &Path, args: &[&str]) -> Result<ElevatedLaunch, ElevationError> {
    let exe = std::env::current_exe().map_err(|e| ElevationError::CurrentExe(e.to_string()))?;
    let allowed = exe
        .parent()
        .ok_or_else(|| ElevationError::CurrentExe("current exe has no parent".into()))?
        .to_path_buf();
    run_elevated_in(binary, args, &allowed)
}

/// Run a binary elevated, restricted to `allowed_dir`. `binary` must be
/// absolute, exist as a regular file, and canonicalize into `allowed_dir`.
pub fn run_elevated_in(
    binary: &Path,
    args: &[&str],
    allowed_dir: &Path,
) -> Result<ElevatedLaunch, ElevationError> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    if !binary.is_absolute() {
        return Err(ElevationError::NotAbsolute(binary.to_path_buf()));
    }

    let canonical = std::fs::canonicalize(binary)
        .map_err(|e| ElevationError::Canonicalize(binary.to_path_buf(), e.to_string()))?;

    let metadata =
        std::fs::metadata(&canonical).map_err(|_| ElevationError::NotAFile(canonical.clone()))?;
    if !metadata.is_file() {
        return Err(ElevationError::NotAFile(canonical));
    }

    let allowed_canonical = std::fs::canonicalize(allowed_dir)
        .map_err(|e| ElevationError::Canonicalize(allowed_dir.to_path_buf(), e.to_string()))?;

    if !canonical.starts_with(&allowed_canonical) {
        return Err(ElevationError::NotAllowed {
            binary: canonical,
            allowed: allowed_canonical,
        });
    }

    let verb: Vec<u16> = OsStr::new("runas\0").encode_wide().collect();
    let binary_str = canonical.to_string_lossy().into_owned();
    let file: Vec<u16> = OsStr::new(&binary_str)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let params_str = args
        .iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let params: Vec<u16> = OsStr::new(&params_str)
        .encode_wide()
        .chain(Some(0))
        .collect();

    #[allow(unsafe_code)]
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };

    if (result as usize) <= 32 {
        Err(ElevationError::ShellExec(result as usize))
    } else {
        Ok(ElevatedLaunch { binary: canonical })
    }
}

/// Quote a single argument for the Windows command line, per the Microsoft
/// C/C++ argument parsing rules.
fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }

    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');

    let mut backslashes: usize = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else if c == '"' {
            for _ in 0..(backslashes * 2 + 1) {
                quoted.push('\\');
            }
            quoted.push('"');
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                quoted.push('\\');
            }
            quoted.push(c);
            backslashes = 0;
        }
    }

    for _ in 0..(backslashes * 2) {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

/// Check if the current process is running with elevated (Administrator)
/// privileges.
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    #[allow(unsafe_code)]
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        result != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_relative_path() {
        let rel = PathBuf::from("app.exe");
        let tmp = std::env::temp_dir();
        let err = run_elevated_in(&rel, &[], &tmp).unwrap_err();
        matches!(err, ElevationError::NotAbsolute(_));
    }

    #[test]
    fn rejects_path_outside_allowlist() {
        // Pick a binary that exists but sits outside our allowed directory.
        let outside = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        if !outside.exists() {
            eprintln!("skip: cmd.exe missing");
            return;
        }
        let tmp = std::env::temp_dir();
        let err = run_elevated_in(&outside, &[], &tmp).unwrap_err();
        matches!(err, ElevationError::NotAllowed { .. });
    }

    #[test]
    fn rejects_missing_file() {
        let allowed = std::env::temp_dir();
        // Path inside allowed dir but does not exist.
        let missing = allowed.join("definitely-not-here.exe");
        let err = run_elevated_in(&missing, &[], &allowed).unwrap_err();
        matches!(
            err,
            ElevationError::Canonicalize(_, _) | ElevationError::NotAFile(_)
        );
    }
}
