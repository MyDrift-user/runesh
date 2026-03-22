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
    let file: Vec<u16> = OsStr::new(&binary.to_string_lossy())
        .encode_wide()
        .chain(Some(0))
        .collect();
    let params_str = args.join(" ");
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

/// Check if the current process is running with elevated privileges.
pub fn is_elevated() -> bool {
    // Simple heuristic: try to write to a protected location
    let test_path = std::path::Path::new("C:\\Windows\\Temp\\runesh_elevation_test");
    match std::fs::write(test_path, "test") {
        Ok(_) => {
            let _ = std::fs::remove_file(test_path);
            true
        }
        Err(_) => false,
    }
}
