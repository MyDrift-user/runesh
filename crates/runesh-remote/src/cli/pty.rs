//! PTY allocation and management using portable-pty.

#[cfg(feature = "cli")]
mod pty_impl {
    use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

    use crate::error::RemoteError;

    /// A PTY session wrapping a master/slave pair with a spawned shell.
    pub struct PtyHandle {
        master: Box<dyn MasterPty + Send>,
        reader: Box<dyn std::io::Read + Send>,
        writer: Box<dyn std::io::Write + Send>,
        child: Box<dyn portable_pty::Child + Send>,
        pub shell: String,
    }

    // SAFETY: PtyHandle fields are all Send. We only access them from one
    // thread at a time via Mutex<SessionState> in the session manager.
    // MasterPty/Child are not Sync by default but we protect access with a mutex.
    unsafe impl Sync for PtyHandle {}

    impl PtyHandle {
        /// Spawn a new PTY with the given shell and size.
        pub fn spawn(
            shell: Option<&str>,
            cols: u16,
            rows: u16,
            cwd: Option<&str>,
            env: &std::collections::HashMap<String, String>,
        ) -> Result<Self, RemoteError> {
            let pty_system = native_pty_system();

            let pair = pty_system
                .openpty(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| RemoteError::Internal(format!("Failed to open PTY: {e}")))?;

            let shell_path = shell.map(String::from).unwrap_or_else(default_shell);

            // Validate shell path: only allow safe filesystem characters
            if !shell_path.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | ':' | '_' | '-')
            }) {
                return Err(RemoteError::NotAllowed(
                    "Shell path contains invalid characters".into(),
                ));
            }

            let mut cmd = CommandBuilder::new(&shell_path);

            if let Some(cwd) = cwd {
                cmd.cwd(cwd);
            }

            for (key, value) in env {
                cmd.env(key, value);
            }

            // Set TERM for proper terminal support
            cmd.env("TERM", "xterm-256color");

            let child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| RemoteError::Internal(format!("Failed to spawn shell: {e}")))?;

            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| RemoteError::Internal(format!("Failed to clone PTY reader: {e}")))?;

            let writer = pair
                .master
                .take_writer()
                .map_err(|e| RemoteError::Internal(format!("Failed to take PTY writer: {e}")))?;

            Ok(Self {
                master: pair.master,
                reader,
                writer,
                child,
                shell: shell_path,
            })
        }

        /// Resize the terminal.
        pub fn resize(&self, cols: u16, rows: u16) -> Result<(), RemoteError> {
            self.master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| RemoteError::Internal(format!("Failed to resize PTY: {e}")))
        }

        /// Write input data to the PTY.
        pub fn write_input(&mut self, data: &[u8]) -> Result<(), RemoteError> {
            use std::io::Write;
            self.writer
                .write_all(data)
                .map_err(|e| RemoteError::Internal(format!("Failed to write to PTY: {e}")))?;
            self.writer
                .flush()
                .map_err(|e| RemoteError::Internal(format!("Failed to flush PTY: {e}")))
        }

        /// Read output from the PTY. Non-blocking: reads whatever is available.
        pub fn read_output(&mut self, buf: &mut [u8]) -> Result<usize, RemoteError> {
            use std::io::Read;
            self.reader.read(buf).map_err(|e| {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    return RemoteError::Internal("No data available".into());
                }
                RemoteError::Internal(format!("Failed to read from PTY: {e}"))
            })
        }

        /// Check if the child process has exited.
        pub fn try_wait(&mut self) -> Option<u32> {
            self.child
                .try_wait()
                .ok()
                .flatten()
                .map(|status| status.exit_code())
        }

        /// Kill the child process.
        pub fn kill(&mut self) {
            let _ = self.child.kill();
        }
    }

    impl Drop for PtyHandle {
        fn drop(&mut self) {
            self.kill();
        }
    }

    /// Get the default shell for the current platform.
    fn default_shell() -> String {
        #[cfg(target_os = "windows")]
        {
            // COMSPEC points at cmd.exe on every current Windows install;
            // fall back to the same rather than to PowerShell so the fallback
            // matches the environment variable's intended value.
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
        }

        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        }
    }
}

#[cfg(feature = "cli")]
pub use pty_impl::PtyHandle;
