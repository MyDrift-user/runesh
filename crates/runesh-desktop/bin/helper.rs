//! `runesh-desktop-helper` — Windows session helper binary.
//!
//! Spawned by a `LocalSystem` service via
//! [`runesh_desktop::session_helper::spawn_in_active_user_session`]
//! under the active user's token. Runs [`run_helper`] which creates
//! the real capture backend inside the user's session and streams
//! frames back over a named pipe.
//!
//! Install it next to the parent binary (e.g. `C:\Program Files\<app>\`)
//! and pass that path to `spawn_in_active_user_session`.
//!
//! Command-line interface:
//!
//! ```text
//! runesh-desktop-helper.exe --capture-pipe \\.\pipe\<name> --display <id>
//! ```
//!
//! No interactive usage — the service is the only legitimate caller.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_code)]

#[cfg(windows)]
fn main() {
    let mut pipe: Option<String> = None;
    let mut display_id: u32 = 0;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--capture-pipe" => {
                pipe = args.next();
            }
            "--display" => {
                display_id = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let Some(pipe) = pipe else {
        eprintln!("missing --capture-pipe <name>");
        std::process::exit(2);
    };

    if let Err(e) = runesh_desktop::session_helper::run_helper(&pipe, display_id) {
        eprintln!("session-helper exited with error: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("runesh-desktop-helper is Windows-only");
    std::process::exit(2);
}
