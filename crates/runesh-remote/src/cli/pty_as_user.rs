//! Spawn a PTY-wrapped process inside a target user's desktop session.
//!
//! The default `PtyHandle::spawn` uses `portable-pty`, which internally
//! calls `CreateProcessW`. When the agent is a `LocalSystem` service,
//! that lands the shell in Session 0 (invisible, no keyboard) with the
//! service's own (very privileged) token. Operators want the shell to
//! feel like the interactive user just typed it, for two reasons:
//!
//! 1. **UX**: the shell sees the user's `%USERPROFILE%`, `%PATH%`, mapped
//!    drives, and proxy settings. A `LocalSystem` shell sees none of those.
//! 2. **Audit**: actions are attributable to the real user, not to the
//!    machine account.
//!
//! This module supplies the missing primitive via ConPTY +
//! `CreateProcessAsUserW`. Two entry points:
//!
//! - [`spawn_as_active_user`] - spawn as whoever is signed in to the active
//!   console session. Used for the "Interactive user" option in the UI.
//! - [`spawn_with_credentials`] - spawn as an explicit `username` + optional
//!   `domain` + `password`. Used for the "Custom user" option.
//!
//! Both return a [`PtyAsUserHandle`] that exposes the same four
//! operations the per-portable-pty [`crate::cli::pty::PtyHandle`] does
//! (`write_input`, `read_output`, `resize`, `try_wait`, `kill`) plus
//! `Drop` cleanup. Read/write are blocking; callers should park the
//! IO loop on a dedicated thread just like they do for portable-pty.
//!
//! Windows-only. On every other platform the module compiles to an
//! empty stub; callers should fall back to `PtyHandle::spawn`.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::mem::{self, ManuallyDrop};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, FALSE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Security::{
    LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, LogonUserW, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    STARTUPINFOW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::error::{PtyStage, RemoteError};

/// A running pseudo-console process owned by another user.
///
/// All IO is blocking; the design matches [`crate::cli::pty::PtyHandle`]
/// so `rumi-agent`'s `shell.rs` can switch backends behind a single
/// enum dispatch.
pub struct PtyAsUserHandle {
    hpc: HPCON,
    child: PROCESS_INFORMATION,
    parent_write: HANDLE,
    parent_read: HANDLE,
    // Backing buffer for the proc-thread attribute list. Must outlive
    // the CreateProcessAsUserW call; we carry it along so Drop can
    // DeleteProcThreadAttributeList it.
    attr_list: Vec<u8>,
    env_block: *mut core::ffi::c_void,
    user_token: HANDLE,
    pub shell: String,
}

// SAFETY: Windows HANDLEs and raw env-block pointers carry no thread
// affinity; the OS manages them. Callers still need to serialize
// access to a handle with a Mutex since read/write aren't themselves
// re-entrant safe. This matches what `PtyHandle` documents.
#[allow(unsafe_code)]
unsafe impl Send for PtyAsUserHandle {}
#[allow(unsafe_code)]
unsafe impl Sync for PtyAsUserHandle {}

impl PtyAsUserHandle {
    /// Spawn the given shell as the user currently signed into the
    /// active console session.
    pub fn spawn_as_active_user(
        shell: Option<&str>,
        cols: u16,
        rows: u16,
        cwd: Option<&str>,
        env: &HashMap<String, String>,
    ) -> Result<Self, RemoteError> {
        let token = active_console_user_token()?;
        // spawn_with_token takes ownership of the token — on error we
        // still want it closed.
        Self::spawn_with_token(token, shell, cols, rows, cwd, env)
    }

    /// Spawn the given shell as an explicit user. `domain` may be
    /// `None` for local accounts and MS-account style `user@domain`
    /// names.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_credentials(
        username: &str,
        domain: Option<&str>,
        password: &str,
        shell: Option<&str>,
        cols: u16,
        rows: u16,
        cwd: Option<&str>,
        env: &HashMap<String, String>,
    ) -> Result<Self, RemoteError> {
        let token = logon_user_token(username, domain, password)?;
        Self::spawn_with_token(token, shell, cols, rows, cwd, env)
    }

    /// Resize the pseudo console. Safe to call concurrently with IO.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), RemoteError> {
        resize_hpc(self.hpc, cols, rows)
    }

    /// Blocking write of `data` to the child's stdin.
    pub fn write_input(&mut self, data: &[u8]) -> Result<(), RemoteError> {
        write_pipe(self.parent_write, data)
    }

    /// Blocking read of up to `buf.len()` bytes from the child's
    /// stdout. Returns `Ok(0)` when the child has closed its stdout
    /// (typically because it exited). Intended for a dedicated reader
    /// thread — the same shape `portable-pty` uses.
    pub fn read_output(&mut self, buf: &mut [u8]) -> Result<usize, RemoteError> {
        read_pipe(self.parent_read, buf)
    }

    /// Split this handle into independent reader and writer halves
    /// that can be moved into separate threads. Mirrors
    /// `portable-pty`'s `try_clone_reader` + `take_writer` shape:
    /// park a dedicated thread in `PtyReader::read_output` while the
    /// main loop calls `PtyWriter::write_input` / `resize` as control
    /// frames arrive — a shared-`&mut self` handle can't do that
    /// because the blocking read holds the only mutable borrow.
    ///
    /// All teardown responsibilities move to [`PtyWriter`]: closing
    /// the pseudo console, killing the child, releasing the env
    /// block, dropping the attribute list, and closing the user
    /// token. [`PtyReader`] only owns the parent-side stdout pipe
    /// handle and closes it on drop (which also EOFs any in-flight
    /// read).
    pub fn split(self) -> (PtyReader, PtyWriter) {
        // ManuallyDrop keeps `self`'s Drop from running after we've
        // moved ownership of the fields into the two halves — each
        // half has its own Drop that cleans up exactly the handles
        // it owns.
        let this = ManuallyDrop::new(self);
        let reader = PtyReader {
            // SAFETY: reading non-Copy fields out of ManuallyDrop is
            // the documented pattern; each field is read exactly
            // once and we never touch `this` again.
            parent_read: this.parent_read,
        };
        let writer = PtyWriter {
            hpc: this.hpc,
            child: this.child,
            parent_write: this.parent_write,
            attr_list: unsafe { ptr::read(&this.attr_list) },
            env_block: this.env_block,
            user_token: this.user_token,
            shell: unsafe { ptr::read(&this.shell) },
        };
        (reader, writer)
    }

    /// Non-blocking exit-code probe. Returns `Some(code)` once the
    /// child has exited, `None` while it's still running.
    pub fn try_wait(&mut self) -> Option<u32> {
        // SAFETY: `self.child.hProcess` is valid until Drop.
        #[allow(unsafe_code)]
        let r = unsafe { WaitForSingleObject(self.child.hProcess, 0) };
        if r == WAIT_TIMEOUT {
            return None;
        }
        if r != WAIT_OBJECT_0 {
            return None;
        }
        let mut code: u32 = 0;
        // SAFETY: `self.child.hProcess` is valid; `code` lives across the call.
        #[allow(unsafe_code)]
        if unsafe { GetExitCodeProcess(self.child.hProcess, &mut code) }.is_err() {
            return None;
        }
        Some(code)
    }

    /// Terminate the child. Safe to call multiple times.
    pub fn kill(&mut self) {
        if self.child.hProcess.is_invalid() {
            return;
        }
        // SAFETY: `self.child.hProcess` is valid.
        #[allow(unsafe_code)]
        unsafe {
            let _ = TerminateProcess(self.child.hProcess, 1);
            let _ = WaitForSingleObject(self.child.hProcess, INFINITE);
        }
    }

    fn spawn_with_token(
        token: HANDLE,
        shell: Option<&str>,
        cols: u16,
        rows: u16,
        cwd: Option<&str>,
        env: &HashMap<String, String>,
    ) -> Result<Self, RemoteError> {
        let shell_path = shell
            .map(String::from)
            .unwrap_or_else(|| "cmd.exe".to_string());

        // Reject shell paths with odd characters early so callers
        // can't smuggle flags via the "shell" parameter.
        if !shell_path.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | ':' | '_' | '-' | ' ')
        }) {
            // SAFETY: we own the token.
            #[allow(unsafe_code)]
            unsafe {
                let _ = CloseHandle(token);
            }
            return Err(RemoteError::NotAllowed(
                "Shell path contains invalid characters".into(),
            ));
        }

        // 1. Two anonymous pipes: parent ↔ child.
        //    pty_in_read  goes to the ConPTY  (child stdin side)
        //    pty_in_write stays in the parent (we write to it)
        //    pty_out_read stays in the parent (we read from it)
        //    pty_out_write goes to the ConPTY (child stdout side)
        let (pty_in_read, pty_in_write) = create_pipe_pair()?;
        let (pty_out_read, pty_out_write) = match create_pipe_pair() {
            Ok(pair) => pair,
            Err(e) => {
                close(pty_in_read);
                close(pty_in_write);
                close(token);
                return Err(e);
            }
        };

        // 2. Create the pseudo console. It consumes pty_in_read and
        //    pty_out_write; we close our copies of those afterwards.
        let hpc = match create_pseudo_console(cols, rows, pty_in_read, pty_out_write) {
            Ok(h) => h,
            Err(e) => {
                close(pty_in_read);
                close(pty_in_write);
                close(pty_out_read);
                close(pty_out_write);
                close(token);
                return Err(e);
            }
        };
        // The ConPTY dup'd pty_in_read and pty_out_write; close our
        // originals so EOF propagates cleanly when the child exits.
        close(pty_in_read);
        close(pty_out_write);

        // 3. Build the STARTUPINFOEXW with PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.
        let (mut startup_info, attr_list) = match build_startup_info(hpc) {
            Ok(v) => v,
            Err(e) => {
                close(pty_in_write);
                close(pty_out_read);
                close(token);
                // SAFETY: `hpc` is a valid pseudo console handle.
                #[allow(unsafe_code)]
                unsafe {
                    ClosePseudoConsole(hpc);
                }
                return Err(e);
            }
        };

        // 4. Build the environment block for the user. Overlays
        //    `env` onto it so operator-supplied TERM / locale win.
        let env_block = match build_environment(token, env) {
            Ok(p) => p,
            Err(e) => {
                close(pty_in_write);
                close(pty_out_read);
                close(token);
                free_startup_info(&mut startup_info, attr_list);
                // SAFETY: see above.
                #[allow(unsafe_code)]
                unsafe {
                    ClosePseudoConsole(hpc);
                }
                return Err(e);
            }
        };

        // 5. CreateProcessAsUserW.
        let mut proc_info = PROCESS_INFORMATION::default();
        let mut cmdline = to_wide_mut(&shell_path);
        let cwd_wide_storage = cwd.map(to_wide);
        let cwd_ptr: windows::core::PCWSTR = match &cwd_wide_storage {
            Some(v) => windows::core::PCWSTR(v.as_ptr()),
            None => windows::core::PCWSTR::null(),
        };

        // SAFETY: `cmdline` is a writable, NUL-terminated UTF-16
        // buffer; `startup_info` + its attribute list are populated
        // above; `token` and `env_block` are valid for the call.
        #[allow(unsafe_code)]
        let created = unsafe {
            CreateProcessAsUserW(
                Some(token),
                windows::core::PCWSTR::null(),
                Some(windows::core::PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                Some(env_block),
                cwd_ptr,
                &startup_info.StartupInfo,
                &mut proc_info,
            )
        };

        if let Err(e) = created {
            close(pty_in_write);
            close(pty_out_read);
            close(token);
            free_startup_info(&mut startup_info, attr_list);
            // SAFETY: env_block came from CreateEnvironmentBlock.
            #[allow(unsafe_code)]
            unsafe {
                let _ = DestroyEnvironmentBlock(env_block);
                ClosePseudoConsole(hpc);
            }
            return Err(win_err(PtyStage::CreateProcessAsUser, e));
        }

        Ok(Self {
            hpc,
            child: proc_info,
            parent_write: pty_in_write,
            parent_read: pty_out_read,
            attr_list,
            env_block,
            user_token: token,
            shell: shell_path,
        })
    }
}

impl Drop for PtyAsUserHandle {
    fn drop(&mut self) {
        self.kill();
        // SAFETY: all fields were populated by successful Win32 calls.
        #[allow(unsafe_code)]
        unsafe {
            if !self.child.hProcess.is_invalid() {
                let _ = CloseHandle(self.child.hProcess);
            }
            if !self.child.hThread.is_invalid() {
                let _ = CloseHandle(self.child.hThread);
            }
            ClosePseudoConsole(self.hpc);
            if !self.parent_read.is_invalid() {
                let _ = CloseHandle(self.parent_read);
            }
            if !self.parent_write.is_invalid() {
                let _ = CloseHandle(self.parent_write);
            }
            if !self.user_token.is_invalid() {
                let _ = CloseHandle(self.user_token);
            }
            if !self.env_block.is_null() {
                let _ = DestroyEnvironmentBlock(self.env_block);
            }
            // The attribute list sits in our Vec<u8>; delete it before
            // dropping the backing allocation so Windows doesn't keep
            // a stale pointer.
            let plist = LPPROC_THREAD_ATTRIBUTE_LIST(self.attr_list.as_mut_ptr() as *mut _);
            DeleteProcThreadAttributeList(plist);
        }
    }
}

// ── Split handles ──────────────────────────────────────────────────────────

/// Read half of a [`PtyAsUserHandle`]. Owns only the parent-side
/// stdout pipe handle so a dedicated thread can park in
/// [`PtyReader::read_output`] without blocking the writer. Closing
/// the reader's handle EOFs any in-flight read; the actual teardown
/// of the pseudo console + child process lives on [`PtyWriter`].
pub struct PtyReader {
    parent_read: HANDLE,
}

// SAFETY: see PtyAsUserHandle.
#[allow(unsafe_code)]
unsafe impl Send for PtyReader {}
#[allow(unsafe_code)]
unsafe impl Sync for PtyReader {}

impl PtyReader {
    /// Blocking read of up to `buf.len()` bytes from the child's
    /// stdout. Returns `Ok(0)` on EOF (child exited, or the writer
    /// was dropped and closed the pipe).
    pub fn read_output(&mut self, buf: &mut [u8]) -> Result<usize, RemoteError> {
        read_pipe(self.parent_read, buf)
    }
}

impl Drop for PtyReader {
    fn drop(&mut self) {
        close(self.parent_read);
    }
}

/// Write half of a [`PtyAsUserHandle`]. Owns the pseudo console
/// plus every resource whose lifetime must extend until the child
/// process exits: the parent-side stdin pipe, the user token, the
/// env block, the proc-thread attribute list, and the child's
/// process + main-thread handles.
pub struct PtyWriter {
    hpc: HPCON,
    child: PROCESS_INFORMATION,
    parent_write: HANDLE,
    attr_list: Vec<u8>,
    env_block: *mut core::ffi::c_void,
    user_token: HANDLE,
    pub shell: String,
}

// SAFETY: see PtyAsUserHandle.
#[allow(unsafe_code)]
unsafe impl Send for PtyWriter {}
#[allow(unsafe_code)]
unsafe impl Sync for PtyWriter {}

impl PtyWriter {
    /// Blocking write of `data` to the child's stdin.
    pub fn write_input(&mut self, data: &[u8]) -> Result<(), RemoteError> {
        write_pipe(self.parent_write, data)
    }

    /// Resize the pseudo console.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), RemoteError> {
        resize_hpc(self.hpc, cols, rows)
    }

    /// Non-blocking exit-code probe. Returns `Some(code)` once the
    /// child has exited, `None` while it's still running.
    pub fn try_wait(&mut self) -> Option<u32> {
        try_wait_child(&self.child)
    }

    /// Terminate the child. Safe to call multiple times.
    pub fn kill(&mut self) {
        kill_child(&self.child);
    }
}

impl Drop for PtyWriter {
    fn drop(&mut self) {
        self.kill();
        // SAFETY: all fields were populated by a successful spawn.
        #[allow(unsafe_code)]
        unsafe {
            if !self.child.hProcess.is_invalid() {
                let _ = CloseHandle(self.child.hProcess);
            }
            if !self.child.hThread.is_invalid() {
                let _ = CloseHandle(self.child.hThread);
            }
            ClosePseudoConsole(self.hpc);
            if !self.parent_write.is_invalid() {
                let _ = CloseHandle(self.parent_write);
            }
            if !self.user_token.is_invalid() {
                let _ = CloseHandle(self.user_token);
            }
            if !self.env_block.is_null() {
                let _ = DestroyEnvironmentBlock(self.env_block);
            }
            if !self.attr_list.is_empty() {
                let plist = LPPROC_THREAD_ATTRIBUTE_LIST(self.attr_list.as_mut_ptr() as *mut _);
                DeleteProcThreadAttributeList(plist);
            }
        }
    }
}

// ── Shared IO helpers ──────────────────────────────────────────────────────

fn write_pipe(pipe: HANDLE, data: &[u8]) -> Result<(), RemoteError> {
    let mut written: u32 = 0;
    // SAFETY: `pipe` is a valid writable pipe handle for the caller's
    // lifetime; `data` + `written` outlive the call.
    #[allow(unsafe_code)]
    unsafe { WriteFile(pipe, Some(data), Some(&mut written), None) }
        .map_err(|e| win_err(PtyStage::WriteFile, e))?;
    if written as usize != data.len() {
        return Err(RemoteError::Pty {
            stage: PtyStage::WriteFile,
            source: std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                format!("short write to child stdin: {written}/{}", data.len()),
            ),
        });
    }
    Ok(())
}

fn read_pipe(pipe: HANDLE, buf: &mut [u8]) -> Result<usize, RemoteError> {
    let mut read_count: u32 = 0;
    // SAFETY: `pipe` is a valid readable pipe handle; `buf` + `read_count`
    // outlive the call.
    #[allow(unsafe_code)]
    let r = unsafe { ReadFile(pipe, Some(buf), Some(&mut read_count), None) };
    match r {
        Ok(()) => Ok(read_count as usize),
        Err(e) => {
            // The child closing stdout surfaces as ERROR_BROKEN_PIPE.
            // Report as clean EOF so the caller's read loop can exit
            // without a spurious error badge.
            let code = e.code().0 as u32;
            if code == ERROR_BROKEN_PIPE.0 {
                Ok(0)
            } else {
                Err(win_err(PtyStage::ReadFile, e))
            }
        }
    }
}

fn resize_hpc(hpc: HPCON, cols: u16, rows: u16) -> Result<(), RemoteError> {
    let size = COORD {
        X: cols as i16,
        Y: rows as i16,
    };
    // SAFETY: `hpc` is valid for the caller's lifetime.
    #[allow(unsafe_code)]
    unsafe { ResizePseudoConsole(hpc, size) }.map_err(|e| win_err(PtyStage::ResizePseudoConsole, e))
}

fn try_wait_child(child: &PROCESS_INFORMATION) -> Option<u32> {
    // SAFETY: `child.hProcess` is a valid process handle owned by the
    // caller.
    #[allow(unsafe_code)]
    let r = unsafe { WaitForSingleObject(child.hProcess, 0) };
    if r != WAIT_OBJECT_0 {
        return None;
    }
    let mut code: u32 = 0;
    #[allow(unsafe_code)]
    if unsafe { GetExitCodeProcess(child.hProcess, &mut code) }.is_err() {
        return None;
    }
    Some(code)
}

fn kill_child(child: &PROCESS_INFORMATION) {
    if child.hProcess.is_invalid() {
        return;
    }
    // SAFETY: `child.hProcess` is valid.
    #[allow(unsafe_code)]
    unsafe {
        let _ = TerminateProcess(child.hProcess, 1);
        let _ = WaitForSingleObject(child.hProcess, INFINITE);
    }
}

/// Wrap a `windows::core::Error` into a structured `RemoteError::Pty`.
fn win_err(stage: PtyStage, e: windows::core::Error) -> RemoteError {
    RemoteError::Pty {
        stage,
        source: std::io::Error::from_raw_os_error(e.code().0),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn create_pipe_pair() -> Result<(HANDLE, HANDLE), RemoteError> {
    let mut r = HANDLE::default();
    let mut w = HANDLE::default();
    let sa = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        // Handles must not be inherited except via the explicit
        // STARTUPINFOEXW handoff we'll do below. Pipes passed to
        // ConPTY are dup'd internally.
        bInheritHandle: FALSE,
    };
    // SAFETY: `r`, `w`, and `sa` are valid for the duration of the call.
    #[allow(unsafe_code)]
    unsafe { CreatePipe(&mut r, &mut w, Some(&sa), 0) }
        .map_err(|e| win_err(PtyStage::CreatePipe, e))?;
    Ok((r, w))
}

fn create_pseudo_console(
    cols: u16,
    rows: u16,
    h_input: HANDLE,
    h_output: HANDLE,
) -> Result<HPCON, RemoteError> {
    let size = COORD {
        X: cols as i16,
        Y: rows as i16,
    };
    // SAFETY: handles are valid; `size` is POD.
    #[allow(unsafe_code)]
    let hpc = unsafe { CreatePseudoConsole(size, h_input, h_output, 0) }
        .map_err(|e| win_err(PtyStage::CreatePseudoConsole, e))?;
    Ok(hpc)
}

/// Allocate and initialize a STARTUPINFOEXW with the
/// PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE attribute. Returns both the
/// `STARTUPINFOEXW` (by value) and the backing byte vector for its
/// attribute list; the caller is responsible for keeping the Vec alive
/// until CreateProcessAsUserW returns and calling
/// `DeleteProcThreadAttributeList` on drop.
fn build_startup_info(hpc: HPCON) -> Result<(STARTUPINFOEXW, Vec<u8>), RemoteError> {
    let mut size: usize = 0;
    // First call: ask Windows how big the attribute list needs to be.
    // This is documented to fail with ERROR_INSUFFICIENT_BUFFER; we
    // ignore the error and trust the out-param.
    // SAFETY: `size` is valid for the call; null list + 0 attribute
    // count is the documented "size query" form.
    #[allow(unsafe_code)]
    unsafe {
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
    }
    if size == 0 {
        return Err(RemoteError::Pty {
            stage: PtyStage::InitializeProcThreadAttributeList,
            source: std::io::Error::other("returned zero size"),
        });
    }

    let mut buf = vec![0u8; size];
    let plist = LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr() as *mut _);
    // SAFETY: `buf` is sized per Windows' request; `plist` points into it.
    #[allow(unsafe_code)]
    unsafe { InitializeProcThreadAttributeList(Some(plist), 1, None, &mut size) }
        .map_err(|e| win_err(PtyStage::InitializeProcThreadAttributeList, e))?;

    // SAFETY: `plist` is initialized; `hpc` points at a live HPCON
    // that outlives the attribute list (we keep it in Self).
    #[allow(unsafe_code)]
    unsafe {
        UpdateProcThreadAttribute(
            plist,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            Some(&hpc as *const HPCON as *const core::ffi::c_void),
            mem::size_of::<HPCON>(),
            None,
            None,
        )
    }
    .map_err(|e| win_err(PtyStage::UpdateProcThreadAttribute, e))?;

    let mut si = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: mem::size_of::<STARTUPINFOEXW>() as u32,
            dwFlags: STARTF_USESTDHANDLES,
            ..Default::default()
        },
        lpAttributeList: plist,
    };
    // Mute unused-field warnings if any.
    si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    Ok((si, buf))
}

fn free_startup_info(si: &mut STARTUPINFOEXW, mut attr_list: Vec<u8>) {
    // SAFETY: attribute list was produced by InitializeProcThreadAttributeList.
    #[allow(unsafe_code)]
    unsafe {
        let plist = LPPROC_THREAD_ATTRIBUTE_LIST(attr_list.as_mut_ptr() as *mut _);
        DeleteProcThreadAttributeList(plist);
    }
    si.lpAttributeList = LPPROC_THREAD_ATTRIBUTE_LIST(ptr::null_mut());
}

fn build_environment(
    token: HANDLE,
    overlay: &HashMap<String, String>,
) -> Result<*mut core::ffi::c_void, RemoteError> {
    let mut block: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: `block` and `token` are valid for the call. DoNotInherit=false
    // means the block inherits from the user's profile.
    #[allow(unsafe_code)]
    unsafe { CreateEnvironmentBlock(&mut block, Some(token), false) }
        .map_err(|e| win_err(PtyStage::CreateEnvironmentBlock, e))?;

    if overlay.is_empty() {
        return Ok(block);
    }

    // Overlay the caller's env by rewriting the block. The format is
    // a sequence of NUL-terminated "K=V" UTF-16 strings, terminated
    // by an extra NUL. Parse into a map, overlay, serialize back, and
    // release the original.
    let mut env = parse_environment_block(block);
    for (k, v) in overlay {
        env.insert(k.clone(), v.clone());
    }
    let serialized = serialize_environment_block(&env);
    // SAFETY: `block` came from CreateEnvironmentBlock.
    #[allow(unsafe_code)]
    unsafe {
        let _ = DestroyEnvironmentBlock(block);
    }
    Ok(serialized)
}

#[allow(unsafe_code)]
fn parse_environment_block(block: *mut core::ffi::c_void) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if block.is_null() {
        return out;
    }
    let mut p = block as *const u16;
    loop {
        // SAFETY: the block is a valid UTF-16 double-NUL-terminated
        // list produced by CreateEnvironmentBlock.
        let len = unsafe { wcslen(p) };
        if len == 0 {
            break;
        }
        let slice = unsafe { std::slice::from_raw_parts(p, len) };
        let s = OsString::from_wide(slice);
        if let Some(entry) = s.to_str()
            && let Some((k, v)) = entry.split_once('=')
        {
            out.insert(k.to_string(), v.to_string());
        }
        // Advance past string + NUL terminator.
        p = unsafe { p.add(len + 1) };
    }
    out
}

#[allow(unsafe_code)]
unsafe fn wcslen(mut p: *const u16) -> usize {
    let mut n = 0usize;
    while unsafe { *p } != 0 {
        n += 1;
        p = unsafe { p.add(1) };
    }
    n
}

fn serialize_environment_block(env: &HashMap<String, String>) -> *mut core::ffi::c_void {
    let mut buf: Vec<u16> = Vec::new();
    // Keys must come out sorted for documentation's sake, though
    // CreateProcessAsUserW itself doesn't require it.
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    for k in keys {
        if let Some(v) = env.get(k) {
            buf.extend(k.encode_utf16());
            buf.push(b'=' as u16);
            buf.extend(v.encode_utf16());
            buf.push(0);
        }
    }
    buf.push(0);

    // Leak the Vec to a raw pointer — Windows expects contiguous
    // memory that lives until CreateProcessAsUserW returns, and Drop
    // frees it via `Vec::from_raw_parts` reconstruction below.
    let boxed = buf.into_boxed_slice();
    Box::into_raw(boxed) as *mut core::ffi::c_void
}

// Small helper so the close path stays terse.
fn close(h: HANDLE) {
    if h.is_invalid() {
        return;
    }
    // SAFETY: called only on handles we own.
    #[allow(unsafe_code)]
    unsafe {
        let _ = CloseHandle(h);
    }
}

// ── Token acquisition ──────────────────────────────────────────────────────

fn active_console_user_token() -> Result<HANDLE, RemoteError> {
    // SAFETY: no parameters. Returns the session id of the active
    // console; -1 (u32::MAX) when there isn't one.
    #[allow(unsafe_code)]
    let session_id = unsafe { WTSGetActiveConsoleSessionId() };
    if session_id == u32::MAX {
        return Err(RemoteError::Pty {
            stage: PtyStage::NoActiveConsoleSession,
            source: std::io::Error::other("no user signed in locally"),
        });
    }
    let mut token = HANDLE::default();
    // SAFETY: `token` is valid for the call.
    #[allow(unsafe_code)]
    unsafe { WTSQueryUserToken(session_id, &mut token) }
        .map_err(|e| win_err(PtyStage::QueryUserToken, e))?;
    Ok(token)
}

fn logon_user_token(
    username: &str,
    domain: Option<&str>,
    password: &str,
) -> Result<HANDLE, RemoteError> {
    let user_w = to_wide(username);
    let domain_w = domain.map(to_wide);
    let password_w = to_wide(password);
    let user_ptr = windows::core::PCWSTR(user_w.as_ptr());
    let domain_ptr = match &domain_w {
        Some(v) => windows::core::PCWSTR(v.as_ptr()),
        None => windows::core::PCWSTR::null(),
    };
    let password_ptr = windows::core::PCWSTR(password_w.as_ptr());

    let mut token = HANDLE::default();
    // SAFETY: all three strings are NUL-terminated UTF-16 buffers that
    // outlive the call; `token` is valid for the call.
    #[allow(unsafe_code)]
    unsafe {
        LogonUserW(
            user_ptr,
            domain_ptr,
            password_ptr,
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    }
    .map_err(|e| win_err(PtyStage::LogonUser, e))?;
    Ok(token)
}

// ── Misc utilities ─────────────────────────────────────────────────────────

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn to_wide_mut(s: &str) -> Vec<u16> {
    // Separate from `to_wide` so call sites that need a `PWSTR` can
    // pass a mutable pointer without accidentally sharing buffers.
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Public entry point for the "Interactive user" operator option.
pub fn spawn_as_active_user(
    shell: Option<&str>,
    cols: u16,
    rows: u16,
    cwd: Option<&str>,
    env: &HashMap<String, String>,
) -> Result<PtyAsUserHandle, RemoteError> {
    PtyAsUserHandle::spawn_as_active_user(shell, cols, rows, cwd, env)
}

/// Public entry point for the "Custom user" operator option.
#[allow(clippy::too_many_arguments)]
pub fn spawn_with_credentials(
    username: &str,
    domain: Option<&str>,
    password: &str,
    shell: Option<&str>,
    cols: u16,
    rows: u16,
    cwd: Option<&str>,
    env: &HashMap<String, String>,
) -> Result<PtyAsUserHandle, RemoteError> {
    PtyAsUserHandle::spawn_with_credentials(username, domain, password, shell, cols, rows, cwd, env)
}
