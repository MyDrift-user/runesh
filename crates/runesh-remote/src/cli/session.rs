//! Terminal session lifecycle management.

#[cfg(feature = "cli")]
mod session_impl {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Instant;

    use tokio::sync::Mutex;

    use crate::cli::audit::AuditLogger;
    use crate::cli::pty::PtyHandle;
    use crate::error::RemoteError;
    use crate::protocol::SessionInfo;

    /// Environment variables an attacker must never be able to inject into
    /// the PTY. Loader hijacks, module path pollution, and arbitrary-command
    /// tricks via `NODE_OPTIONS` / `PATH` all live here.
    const DENYLIST_ENV_KEYS: &[&str] = &[
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FORCE_FLAT_NAMESPACE",
        "DYLD_IMAGE_SUFFIX",
        "PYTHONPATH",
        "PERL5LIB",
        "RUBYLIB",
        "NODE_OPTIONS",
        "PATH",
    ];

    /// Strip any denylisted env keys. Comparison is case-insensitive on
    /// Windows (where env var names are case-insensitive) and case-sensitive
    /// on Unix.
    fn filter_env(input: &HashMap<String, String>) -> HashMap<String, String> {
        input
            .iter()
            .filter(|(k, _)| {
                !DENYLIST_ENV_KEYS.iter().any(|denied| {
                    #[cfg(windows)]
                    {
                        denied.eq_ignore_ascii_case(k.as_str())
                    }
                    #[cfg(not(windows))]
                    {
                        *denied == k.as_str()
                    }
                })
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Validate that `cwd` resolves to a path inside `sandbox_root`.
    fn check_cwd(cwd: &str, sandbox_root: &Path) -> Result<PathBuf, RemoteError> {
        if cwd.contains('\0') {
            return Err(RemoteError::BadRequest(
                "cwd must not contain null bytes".into(),
            ));
        }
        let requested = Path::new(cwd);
        let full = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            sandbox_root.join(requested)
        };
        let canon = full.canonicalize().map_err(|e| {
            RemoteError::BadRequest(format!("cwd does not exist or is invalid: {e}"))
        })?;
        let canon_root = sandbox_root.canonicalize().map_err(|e| {
            RemoteError::Internal(format!("Failed to canonicalize sandbox root: {e}"))
        })?;
        if !canon.starts_with(&canon_root) {
            return Err(RemoteError::NotAllowed(
                "cwd must be inside the sandbox root".into(),
            ));
        }
        Ok(canon)
    }

    /// Configuration for the session manager.
    #[derive(Debug, Clone)]
    pub struct SessionConfig {
        /// Maximum concurrent terminal sessions.
        pub max_sessions: usize,
        /// Session idle timeout in seconds.
        pub idle_timeout_secs: u64,
        /// Allowed shells (empty = all allowed).
        pub allowed_shells: Vec<String>,
        /// Root directory that client-supplied `cwd` must be inside. When
        /// `None`, `cwd` is refused entirely (safe default for multi-tenant
        /// deployments).
        pub sandbox_root: Option<PathBuf>,
    }

    impl Default for SessionConfig {
        fn default() -> Self {
            Self {
                max_sessions: 10,
                idle_timeout_secs: 1800, // 30 minutes
                allowed_shells: Vec::new(),
                sandbox_root: None,
            }
        }
    }

    /// Manages terminal sessions with resource limits and audit logging.
    pub struct SessionManager {
        sessions: Arc<Mutex<HashMap<String, SessionState>>>,
        config: SessionConfig,
        audit: Arc<AuditLogger>,
    }

    struct SessionState {
        pty: PtyHandle,
        shell: String,
        created_at: Instant,
        last_activity: Instant,
        cols: u16,
        rows: u16,
    }

    impl SessionManager {
        pub fn new(config: SessionConfig, audit: Arc<AuditLogger>) -> Self {
            Self {
                sessions: Arc::new(Mutex::new(HashMap::new())),
                config,
                audit,
            }
        }

        /// Open a new terminal session.
        pub async fn open(
            &self,
            shell: Option<&str>,
            cols: u16,
            rows: u16,
            cwd: Option<&str>,
            env: &HashMap<String, String>,
            user: Option<&str>,
        ) -> Result<(String, String), RemoteError> {
            // Check session limit
            let sessions = self.sessions.lock().await;
            if sessions.len() >= self.config.max_sessions {
                return Err(RemoteError::MaxSessions);
            }
            drop(sessions);

            // Validate shell if allowlist is configured
            if let Some(shell) = shell
                && !self.config.allowed_shells.is_empty()
                && !self.config.allowed_shells.iter().any(|s| s == shell)
            {
                return Err(RemoteError::NotAllowed(format!(
                    "Shell '{}' is not in the allowed list",
                    shell
                )));
            }

            // Sanitize env: drop anything that could be used to hijack the
            // loader or pollute module search paths.
            let safe_env = filter_env(env);

            // Validate cwd. Reject entirely if no sandbox root is configured.
            let safe_cwd: Option<String> = match cwd {
                Some(c) => {
                    let root = self.config.sandbox_root.as_ref().ok_or_else(|| {
                        RemoteError::NotAllowed(
                            "cwd is not allowed: no sandbox root configured".into(),
                        )
                    })?;
                    Some(check_cwd(c, root)?.to_string_lossy().to_string())
                }
                None => None,
            };

            let pty = tokio::task::spawn_blocking({
                let shell = shell.map(String::from);
                let cwd = safe_cwd;
                let env = safe_env;
                move || PtyHandle::spawn(shell.as_deref(), cols, rows, cwd.as_deref(), &env)
            })
            .await
            .map_err(|e| RemoteError::Internal(format!("Spawn task failed: {e}")))??;

            let session_id = uuid::Uuid::new_v4().to_string();
            let shell_name = pty.shell.clone();

            self.audit
                .log_session_open(&session_id, &shell_name, user)
                .await;

            self.sessions.lock().await.insert(
                session_id.clone(),
                SessionState {
                    pty,
                    shell: shell_name.clone(),
                    created_at: Instant::now(),
                    last_activity: Instant::now(),
                    cols,
                    rows,
                },
            );

            Ok((session_id, shell_name))
        }

        /// Send input to a session.
        pub async fn input(&self, session_id: &str, data: &[u8]) -> Result<(), RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| RemoteError::SessionNotFound(session_id.into()))?;
            session.last_activity = Instant::now();

            // PTY write is a blocking I/O call; execute it immediately
            // but keep lock scope minimal. The write itself is fast (kernel buffer).
            session.pty.write_input(data)
            // Lock is released here at end of function.
        }

        /// Read output from a session.
        pub async fn read_output(
            &self,
            session_id: &str,
            buf: &mut [u8],
        ) -> Result<usize, RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| RemoteError::SessionNotFound(session_id.into()))?;
            session.last_activity = Instant::now();

            // PTY read is non-blocking (reads whatever is available).
            session.pty.read_output(buf)
        }

        /// Resize a session's terminal.
        pub async fn resize(
            &self,
            session_id: &str,
            cols: u16,
            rows: u16,
        ) -> Result<(), RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| RemoteError::SessionNotFound(session_id.into()))?;

            session.cols = cols;
            session.rows = rows;
            session.pty.resize(cols, rows)
        }

        /// Close a session.
        pub async fn close(
            &self,
            session_id: &str,
            user: Option<&str>,
        ) -> Result<Option<u32>, RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let mut session = sessions
                .remove(session_id)
                .ok_or_else(|| RemoteError::SessionNotFound(session_id.into()))?;

            let exit_code = session.pty.try_wait();
            session.pty.kill();

            self.audit
                .log_session_close(session_id, exit_code, user)
                .await;

            Ok(exit_code)
        }

        /// List active sessions.
        pub async fn list_sessions(&self) -> Vec<SessionInfo> {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, state)| {
                    let created_at = chrono::Utc::now()
                        - chrono::Duration::seconds(state.created_at.elapsed().as_secs() as i64);
                    let last_activity = chrono::Utc::now()
                        - chrono::Duration::seconds(state.last_activity.elapsed().as_secs() as i64);

                    SessionInfo {
                        session_id: id.clone(),
                        shell: state.shell.clone(),
                        created_at: created_at.to_rfc3339(),
                        last_activity: last_activity.to_rfc3339(),
                        cols: state.cols,
                        rows: state.rows,
                    }
                })
                .collect()
        }

        /// Close all idle sessions that have exceeded the timeout.
        pub async fn cleanup_idle(&self) {
            let timeout = std::time::Duration::from_secs(self.config.idle_timeout_secs);
            let mut sessions = self.sessions.lock().await;

            let idle_keys: Vec<String> = sessions
                .iter()
                .filter(|(_, state)| state.last_activity.elapsed() > timeout)
                .map(|(key, _)| key.clone())
                .collect();

            for key in idle_keys {
                if let Some(mut session) = sessions.remove(&key) {
                    session.pty.kill();
                    tracing::info!(session_id = %key, "Closed idle terminal session");
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn env_denylist_strips_ld_preload_and_path() {
            let mut env = HashMap::new();
            env.insert("LD_PRELOAD".into(), "/tmp/evil.so".into());
            env.insert("PATH".into(), "/tmp".into());
            env.insert("MY_VAR".into(), "ok".into());
            env.insert("PYTHONPATH".into(), "/evil".into());

            let filtered = filter_env(&env);
            assert!(!filtered.contains_key("LD_PRELOAD"));
            assert!(!filtered.contains_key("PATH"));
            assert!(!filtered.contains_key("PYTHONPATH"));
            assert_eq!(filtered.get("MY_VAR").map(String::as_str), Some("ok"));
        }

        #[test]
        fn cwd_outside_sandbox_rejected() {
            let tmp =
                std::env::temp_dir().join(format!("runesh-remote-cwd-{}", std::process::id()));
            std::fs::create_dir_all(&tmp).unwrap();

            // Absolute outside path should fail.
            let res = check_cwd("/", &tmp);
            assert!(res.is_err());
            let _ = std::fs::remove_dir_all(&tmp);
        }

        #[test]
        fn cwd_inside_sandbox_accepted() {
            let tmp =
                std::env::temp_dir().join(format!("runesh-remote-cwd-ok-{}", std::process::id()));
            let sub = tmp.join("sub");
            std::fs::create_dir_all(&sub).unwrap();

            let res = check_cwd("sub", &tmp);
            assert!(res.is_ok(), "{res:?}");
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
}

#[cfg(feature = "cli")]
pub use session_impl::{SessionConfig, SessionManager};
