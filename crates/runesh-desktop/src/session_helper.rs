//! Run the Windows capture backend inside the interactive user's
//! session.
//!
//! Why this exists
//! ===============
//! `IDXGIOutput1::DuplicateOutput` binds to the calling process's
//! logon session and only succeeds from inside the interactive
//! desktop. A `LocalSystem` agent running in Session 0 cannot use
//! it (returns `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` /
//! `E_ACCESSDENIED` — we surface that as
//! [`crate::DesktopError::RequiresInteractiveSession`]). Every
//! mainstream remote-desktop product on Windows (TeamViewer,
//! AnyDesk, RustDesk, Microsoft Quick Assist) works around this by
//! spawning a small "session helper" process inside the logged-in
//! user's session and proxying frames over IPC.
//!
//! Shape
//! =====
//! * [`spawn_in_active_user_session`] looks up the active console
//!   user's primary token via `WTSQueryUserToken`, builds their
//!   environment with `CreateEnvironmentBlock`, creates an anonymous
//!   [`NAMED_PIPE`], and launches the helper binary with
//!   `CreateProcessAsUserW`. Returns a [`SessionCaptureProxy`] that
//!   reads frames from the pipe.
//! * The helper binary calls [`run_helper`] with the pipe name it
//!   received on its command line. The helper creates a real
//!   [`crate::capture::ScreenCapture`] (DXGI — works now because we're
//!   in the user's session) and serves requests in a loop.
//!
//! Wire protocol
//! =============
//! Fixed binary framing, no serde. Each pipe direction is a sequence
//! of length-prefixed messages. Little-endian. For a frame carrying
//! pixel bytes, the bytes follow the header without further framing.
//!
//! ```text
//! Client -> Helper:
//!   u8 kind:
//!     0 = Hello    { u32 display_id }
//!     1 = Capture  { }
//!     2 = Resize   { u32 cols, u32 rows }    (reserved; not used for capture)
//!     3 = Close    { }
//!
//! Helper -> Client:
//!   u8 kind:
//!     0 = HelloOk  { u32 width, u32 height }
//!     1 = Frame    { u32 width, u32 height, u64 ts_ms, u32 data_len, [data] }
//!     2 = Err      { u32 msg_len, [utf8] }
//! ```

use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    ReadFile, WriteFile,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::RemoteDesktop::{
    WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW, WTSActive,
    WTSEnumerateSessionsW, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQueryUserToken,
};
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION, STARTUPINFOW,
    TerminateProcess, WaitForSingleObject,
};

use crate::capture::{CapturedFrame, ScreenCapture};
use crate::error::DesktopError;

/// A capturer that lives in another process. Implements the same
/// [`ScreenCapture`] interface, so callers can drop it into any
/// existing pipeline without changes.
pub struct SessionCaptureProxy {
    pipe: HANDLE,
    child: PROCESS_INFORMATION,
    user_token: HANDLE,
    env_block: *mut core::ffi::c_void,
    width: u32,
    height: u32,
}

// SAFETY: HANDLEs and env-block raw pointers have no thread affinity;
// Windows manages the handles. The struct is single-owner so no cross-
// thread synchronization is needed beyond what `ScreenCapture` already
// requires from callers.
#[allow(unsafe_code)]
unsafe impl Send for SessionCaptureProxy {}

impl ScreenCapture for SessionCaptureProxy {
    fn capture_frame(&mut self) -> Result<CapturedFrame, DesktopError> {
        self.request_capture()
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl SessionCaptureProxy {
    fn request_capture(&mut self) -> Result<CapturedFrame, DesktopError> {
        // Client -> helper: kind=1 (Capture), no payload.
        write_all(self.pipe, &[1u8])?;
        let kind = read_u8(self.pipe)?;
        match kind {
            1 => {
                let width = read_u32(self.pipe)?;
                let height = read_u32(self.pipe)?;
                let ts = read_u64(self.pipe)?;
                let data_len = read_u32(self.pipe)? as usize;
                let mut data = vec![0u8; data_len];
                read_exact(self.pipe, &mut data)?;
                Ok(CapturedFrame {
                    width,
                    height,
                    timestamp: ts,
                    data,
                })
            }
            2 => {
                let msg_len = read_u32(self.pipe)? as usize;
                let mut buf = vec![0u8; msg_len];
                read_exact(self.pipe, &mut buf)?;
                Err(DesktopError::Capture(
                    String::from_utf8_lossy(&buf).into_owned(),
                ))
            }
            other => Err(DesktopError::Capture(format!(
                "session helper sent unknown response kind {other}"
            ))),
        }
    }
}

impl Drop for SessionCaptureProxy {
    fn drop(&mut self) {
        // Best-effort: tell the helper to close, then terminate if it
        // doesn't exit promptly. Releasing the pipe EOFs its reads.
        let _ = write_all(self.pipe, &[3u8]);
        // SAFETY: handles / pointers are either valid or INVALID/NULL.
        #[allow(unsafe_code)]
        unsafe {
            if !self.child.hProcess.is_invalid() {
                let _ = WaitForSingleObject(self.child.hProcess, 500);
                let _ = TerminateProcess(self.child.hProcess, 1);
                let _ = CloseHandle(self.child.hProcess);
            }
            if !self.child.hThread.is_invalid() {
                let _ = CloseHandle(self.child.hThread);
            }
            if !self.pipe.is_invalid() {
                let _ = CloseHandle(self.pipe);
            }
            if !self.user_token.is_invalid() {
                let _ = CloseHandle(self.user_token);
            }
            if !self.env_block.is_null() {
                let _ = DestroyEnvironmentBlock(self.env_block);
            }
        }
    }
}

/// Spawn `helper_exe` inside the logged-in user's session and return
/// a proxy that delivers frames captured from that session.
///
/// `helper_exe` must be an absolute path to a binary that calls
/// [`run_helper`]. Typical deployment: consumers ship
/// `runesh-desktop-helper.exe` alongside their service binary.
pub fn spawn_in_active_user_session(
    helper_exe: &Path,
    display_id: u32,
) -> Result<SessionCaptureProxy, DesktopError> {
    let pipe_name = unique_pipe_name();
    // 1. Create the server end of the named pipe. The helper will
    //    connect to the client end.
    let pipe = create_server_pipe(&pipe_name)?;

    // 2. Acquire a primary token for the active console user.
    let token = active_console_user_token()?;

    // 3. Build the user's environment block. Failure here is fatal —
    //    the helper needs PATH / USERPROFILE / etc.
    let env_block = match build_user_environment(token) {
        Ok(p) => p,
        Err(e) => {
            close_handle(pipe);
            close_handle(token);
            return Err(e);
        }
    };

    // 4. CreateProcessAsUserW. Command line is
    //    `"<helper_exe>" --capture-pipe <pipe_name> --display <id>`.
    let cmdline = format!(
        "\"{}\" --capture-pipe {} --display {}",
        helper_exe.display(),
        pipe_name,
        display_id
    );
    let mut cmdline_w = to_wide_mut(&cmdline);

    let startup = STARTUPINFOW {
        cb: mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut proc_info = PROCESS_INFORMATION::default();

    // SAFETY: cmdline_w is a valid NUL-terminated UTF-16 buffer that
    // outlives the call; env_block came from CreateEnvironmentBlock;
    // token is a valid primary token.
    #[allow(unsafe_code)]
    let created = unsafe {
        CreateProcessAsUserW(
            Some(token),
            windows::core::PCWSTR::null(),
            Some(windows::core::PWSTR(cmdline_w.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT,
            Some(env_block),
            windows::core::PCWSTR::null(),
            &startup,
            &mut proc_info,
        )
    };

    if let Err(e) = created {
        close_handle(pipe);
        close_handle(token);
        // SAFETY: env_block came from CreateEnvironmentBlock.
        #[allow(unsafe_code)]
        unsafe {
            let _ = DestroyEnvironmentBlock(env_block);
        }
        return Err(DesktopError::Capture(format!(
            "CreateProcessAsUserW(helper): {e}"
        )));
    }

    // 5. Wait for the helper to connect to our pipe.
    // SAFETY: `pipe` is a valid named-pipe server handle.
    #[allow(unsafe_code)]
    unsafe { ConnectNamedPipe(pipe, None) }
        .map_err(|e| DesktopError::Capture(format!("ConnectNamedPipe(session helper): {e}")))?;

    // 6. Exchange the Hello handshake so we learn the display
    //    dimensions and surface any capture error up front.
    write_all(pipe, &[0u8])?; // Hello
    write_all(pipe, &display_id.to_le_bytes())?;
    let kind = read_u8(pipe)?;
    let (width, height) = match kind {
        0 => (read_u32(pipe)?, read_u32(pipe)?),
        2 => {
            let msg_len = read_u32(pipe)? as usize;
            let mut buf = vec![0u8; msg_len];
            read_exact(pipe, &mut buf)?;
            return Err(DesktopError::Capture(
                String::from_utf8_lossy(&buf).into_owned(),
            ));
        }
        other => {
            return Err(DesktopError::Capture(format!(
                "session helper sent unexpected hello kind {other}"
            )));
        }
    };

    Ok(SessionCaptureProxy {
        pipe,
        child: proc_info,
        user_token: token,
        env_block,
        width,
        height,
    })
}

/// Entry point for the helper binary. Expects `pipe_name` to be the
/// full `\\.\pipe\<name>` string that the service passed via
/// `--capture-pipe`, and `display_id` from `--display`. Blocks until
/// the service disconnects or asks for `Close`.
pub fn run_helper(pipe_name: &str, display_id: u32) -> Result<(), DesktopError> {
    let pipe = connect_client_pipe(pipe_name)?;

    // Hello handshake: read `kind=0 Hello`, then the display id (we
    // already received one as a CLI arg but the service sends one too
    // so we can revalidate without trusting argv).
    let kind = read_u8(pipe)?;
    if kind != 0 {
        return Err(DesktopError::Capture(format!(
            "session helper expected Hello(0), got {kind}"
        )));
    }
    let wire_display_id = read_u32(pipe)?;
    if wire_display_id != display_id {
        // Log-only — the argv value wins. Consumer's IPC is still
        // serving the same display.
        tracing::warn!(
            argv = display_id,
            wire = wire_display_id,
            "display id mismatch between argv and IPC"
        );
    }

    let mut capturer = match crate::capture::create_capturer(display_id) {
        Ok(c) => c,
        Err(e) => {
            send_err(pipe, &format!("create_capturer({display_id}): {e}"));
            close_handle(pipe);
            return Err(e);
        }
    };

    let (w, h) = capturer.dimensions();
    write_all(pipe, &[0u8])?; // HelloOk
    write_all(pipe, &w.to_le_bytes())?;
    write_all(pipe, &h.to_le_bytes())?;

    // EOF on read_u8 = pipe closed by peer; exit cleanly.
    while let Ok(kind) = read_u8(pipe) {
        match kind {
            1 => match capturer.capture_frame() {
                Ok(frame) => {
                    write_all(pipe, &[1u8])?;
                    write_all(pipe, &frame.width.to_le_bytes())?;
                    write_all(pipe, &frame.height.to_le_bytes())?;
                    write_all(pipe, &frame.timestamp.to_le_bytes())?;
                    write_all(pipe, &(frame.data.len() as u32).to_le_bytes())?;
                    write_all(pipe, &frame.data)?;
                }
                Err(e) => send_err(pipe, &format!("capture_frame: {e}")),
            },
            2 => {
                // Resize — reserved; skip both u32 payloads and
                // no-op. Keeps the protocol extensible.
                let _ = read_u32(pipe)?;
                let _ = read_u32(pipe)?;
            }
            3 => break, // Close
            other => send_err(pipe, &format!("unknown request kind {other}")),
        }
    }

    close_handle(pipe);
    Ok(())
}

// ── IPC I/O helpers ────────────────────────────────────────────────────────

fn write_all(pipe: HANDLE, data: &[u8]) -> Result<(), DesktopError> {
    let mut written = 0u32;
    // SAFETY: `pipe` is a valid pipe handle; `data` + `written` outlive the call.
    #[allow(unsafe_code)]
    unsafe { WriteFile(pipe, Some(data), Some(&mut written), None) }
        .map_err(|e| DesktopError::Internal(format!("WriteFile(session pipe): {e}")))?;
    if written as usize != data.len() {
        return Err(DesktopError::Internal(format!(
            "short write to session pipe: {written}/{}",
            data.len()
        )));
    }
    Ok(())
}

fn read_exact(pipe: HANDLE, buf: &mut [u8]) -> Result<(), DesktopError> {
    let mut filled = 0usize;
    while filled < buf.len() {
        let mut n = 0u32;
        // SAFETY: `pipe` is valid; the subslice outlives the call.
        #[allow(unsafe_code)]
        unsafe { ReadFile(pipe, Some(&mut buf[filled..]), Some(&mut n), None) }
            .map_err(|e| DesktopError::Internal(format!("ReadFile(session pipe): {e}")))?;
        if n == 0 {
            return Err(DesktopError::Internal("session pipe EOF".into()));
        }
        filled += n as usize;
    }
    Ok(())
}

fn read_u8(pipe: HANDLE) -> Result<u8, DesktopError> {
    let mut b = [0u8; 1];
    read_exact(pipe, &mut b)?;
    Ok(b[0])
}

fn read_u32(pipe: HANDLE) -> Result<u32, DesktopError> {
    let mut b = [0u8; 4];
    read_exact(pipe, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(pipe: HANDLE) -> Result<u64, DesktopError> {
    let mut b = [0u8; 8];
    read_exact(pipe, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn send_err(pipe: HANDLE, msg: &str) {
    let bytes = msg.as_bytes();
    let _ = write_all(pipe, &[2u8]);
    let _ = write_all(pipe, &(bytes.len() as u32).to_le_bytes());
    let _ = write_all(pipe, bytes);
}

// ── Named pipe creation ────────────────────────────────────────────────────

fn unique_pipe_name() -> String {
    // Pid + monotonic timestamp makes collisions effectively
    // impossible and keeps the name trivially auditable.
    format!(
        r"\\.\pipe\runesh-desktop-helper-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

fn create_server_pipe(name: &str) -> Result<HANDLE, DesktopError> {
    let name_w = to_wide(name);
    // SAFETY: name_w is a NUL-terminated UTF-16 buffer that outlives
    // the call. The 0 SECURITY_ATTRIBUTES default ACL restricts the
    // pipe to the creator + admins, which is exactly what we want
    // (the helper impersonates the user via CreateProcessAsUserW and
    // still has access because the service creates the pipe).
    #[allow(unsafe_code)]
    let pipe = unsafe {
        CreateNamedPipeW(
            windows::core::PCWSTR(name_w.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1024 * 1024,
            1024 * 1024,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        return Err(DesktopError::Capture(
            "CreateNamedPipeW returned INVALID_HANDLE".into(),
        ));
    }
    Ok(pipe)
}

fn connect_client_pipe(name: &str) -> Result<HANDLE, DesktopError> {
    let name_w = to_wide(name);
    // SAFETY: name_w outlives the call.
    #[allow(unsafe_code)]
    let h = unsafe {
        CreateFileW(
            windows::core::PCWSTR(name_w.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|e| DesktopError::Capture(format!("CreateFileW(client pipe): {e}")))?;
    Ok(h)
}

// ── Token + environment ────────────────────────────────────────────────────

fn active_console_user_token() -> Result<HANDLE, DesktopError> {
    // 1. Fast path: physical console user.
    // SAFETY: no parameters. u32::MAX = no active console.
    #[allow(unsafe_code)]
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    if console != u32::MAX {
        let mut token = HANDLE::default();
        // SAFETY: `token` outlives the call.
        #[allow(unsafe_code)]
        if unsafe { WTSQueryUserToken(console, &mut token) }.is_ok() {
            return Ok(token);
        }
        // Fall through: console is at the login screen / no user.
    }

    // 2. Slow path: enumerate sessions and pick the first Active one
    //    whose token query succeeds. Catches RDP-only hosts (no
    //    console user at all), Fast User Switching, and servers
    //    where an admin is connected over RDP but no one is at the
    //    physical keyboard. `qwinsta` would show this session as
    //    `Active`; earlier code missed it because
    //    `WTSGetActiveConsoleSessionId` only looks at the physical
    //    console.
    let mut info_ptr: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
    let mut count: u32 = 0;
    // SAFETY: out-params live across the call; the sentinel handle
    // targets the current server.
    #[allow(unsafe_code)]
    unsafe {
        WTSEnumerateSessionsW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            0,
            1,
            &mut info_ptr,
            &mut count,
        )
    }
    .map_err(|_| DesktopError::RequiresInteractiveSession)?;

    // RAII guard so WTSFreeMemory fires even on the error paths.
    struct FreeGuard(*mut WTS_SESSION_INFOW);
    impl Drop for FreeGuard {
        fn drop(&mut self) {
            // SAFETY: pointer came from WTSEnumerateSessionsW.
            #[allow(unsafe_code)]
            unsafe {
                WTSFreeMemory(self.0 as _);
            }
        }
    }
    let _guard = FreeGuard(info_ptr);

    // SAFETY: WTSEnumerateSessionsW populated `count` contiguous entries.
    #[allow(unsafe_code)]
    let sessions = unsafe { std::slice::from_raw_parts(info_ptr, count as usize) };
    for s in sessions {
        if s.State != WTS_CONNECTSTATE_CLASS(WTSActive.0) {
            continue;
        }
        let mut token = HANDLE::default();
        // SAFETY: `token` outlives the call.
        #[allow(unsafe_code)]
        if unsafe { WTSQueryUserToken(s.SessionId, &mut token) }.is_ok() {
            return Ok(token);
        }
    }

    Err(DesktopError::RequiresInteractiveSession)
}

fn build_user_environment(token: HANDLE) -> Result<*mut core::ffi::c_void, DesktopError> {
    let mut block: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: `block` + `token` are valid for the call.
    #[allow(unsafe_code)]
    unsafe { CreateEnvironmentBlock(&mut block, Some(token), false) }
        .map_err(|e| DesktopError::Capture(format!("CreateEnvironmentBlock: {e}")))?;
    Ok(block)
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn to_wide_mut(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn close_handle(h: HANDLE) {
    if h.is_invalid() {
        return;
    }
    // SAFETY: only called on handles we own.
    #[allow(unsafe_code)]
    unsafe {
        let _ = CloseHandle(h);
    }
}
