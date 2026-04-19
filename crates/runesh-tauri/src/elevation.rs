//! UAC elevation helpers for Windows.
//!
//! Runs commands with Administrator privileges using the "runas" verb
//! (triggers the Windows UAC prompt).

use std::path::Path;

/// Run a command with elevated (Administrator) privileges on Windows.
///
/// This triggers the Windows UAC prompt. The function returns immediately
/// after launching -- it does NOT wait for the elevated process to finish.
///
/// Usage:
/// ```ignore
/// use runesh_tauri::elevation::run_elevated;
///
/// run_elevated("C:\\Program Files\\MyApp\\agent.exe", &["--install-service"])?;
/// ```
pub fn run_elevated(binary: &Path, args: &[&str]) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let verb: Vec<u16> = OsStr::new("runas\0").encode_wide().collect();
    let binary_str = binary.to_string_lossy().into_owned();
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

    // ShellExecuteW returns a value > 32 on success
    if (result as usize) <= 32 {
        Err(format!(
            "Failed to elevate process (code {})",
            result as usize
        ))
    } else {
        Ok(())
    }
}

/// Quote a single argument for the Windows command line.
///
/// Follows the Microsoft C/C++ argument parsing rules:
/// - If the arg contains spaces, tabs, or quotes, wrap in double quotes
/// - Backslashes before a quote are doubled; trailing backslashes before
///   the closing quote are doubled
fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }

    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');

    let mut backslashes: usize = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else if c == '"' {
            // Double the backslashes before a quote, then escape the quote
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

    // Double trailing backslashes before the closing quote
    for _ in 0..(backslashes * 2) {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

/// Check if the current process is running with elevated (Administrator) privileges.
///
/// Uses the proper Win32 API (OpenProcessToken + GetTokenInformation) instead
/// of heuristics like file write tests.
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

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
