//! Process management utilities for Tauri desktop apps.
//!
//! Find, start, and stop companion processes (agents, services, etc.).

use std::path::PathBuf;
use std::process::Command;

/// Find a binary by name, checking:
/// 1. Same directory as the current executable
/// 2. System PATH
pub fn find_binary(name: &str) -> Option<PathBuf> {
    // Check next to current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
            // Windows: try with .exe extension
            #[cfg(windows)]
            {
                let candidate = dir.join(format!("{name}.exe"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    // Check PATH
    which::which(name).ok()
}

/// Check if a process with the given name is running.
pub fn is_process_running(name: &str) -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.contains(name)
            }
            Err(_) => false,
        }
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("pgrep")
            .arg("-x")
            .arg(name)
            .output();
        matches!(output, Ok(out) if out.status.success())
    }
}

/// Start a process in the background. Returns the child process handle.
///
/// On Windows, uses `CREATE_NO_WINDOW` to prevent a console window.
pub fn start_background(
    binary: &std::path::Path,
    args: &[&str],
) -> Result<std::process::Child, String> {
    let mut cmd = Command::new(binary);
    cmd.args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to start {}: {e}", binary.display()))
}

/// Run a command and capture its output silently.
///
/// On Windows, uses `CREATE_NO_WINDOW` to prevent a console window.
pub fn run_silent(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd.output().map_err(|e| format!("{program}: {e}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
