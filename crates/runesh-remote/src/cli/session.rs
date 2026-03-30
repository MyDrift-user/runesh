//! Terminal session lifecycle management.

#[cfg(feature = "cli")]
mod session_impl {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;

    use tokio::sync::Mutex;

    use crate::cli::audit::AuditLogger;
    use crate::cli::pty::PtyHandle;
    use crate::error::RemoteError;
    use crate::protocol::SessionInfo;

    /// Configuration for the session manager.
    #[derive(Debug, Clone)]
    pub struct SessionConfig {
        /// Maximum concurrent terminal sessions.
        pub max_sessions: usize,
        /// Session idle timeout in seconds.
        pub idle_timeout_secs: u64,
        /// Allowed shells (empty = all allowed).
        pub allowed_shells: Vec<String>,
    }

    impl Default for SessionConfig {
        fn default() -> Self {
            Self {
                max_sessions: 10,
                idle_timeout_secs: 1800, // 30 minutes
                allowed_shells: Vec::new(),
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
            if let Some(shell) = shell {
                if !self.config.allowed_shells.is_empty()
                    && !self.config.allowed_shells.iter().any(|s| s == shell)
                {
                    return Err(RemoteError::NotAllowed(format!(
                        "Shell '{}' is not in the allowed list",
                        shell
                    )));
                }
            }

            let pty = tokio::task::spawn_blocking({
                let shell = shell.map(String::from);
                let cwd = cwd.map(String::from);
                let env = env.clone();
                move || PtyHandle::spawn(shell.as_deref(), cols, rows, cwd.as_deref(), &env)
            })
            .await
            .map_err(|e| RemoteError::Internal(format!("Spawn task failed: {e}")))?
            ?;

            let session_id = uuid::Uuid::new_v4().to_string();
            let shell_name = pty.shell.clone();

            self.audit.log_session_open(&session_id, &shell_name, user).await;

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
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                RemoteError::SessionNotFound(session_id.into())
            })?;

            session.last_activity = Instant::now();
            session.pty.write_input(data)
        }

        /// Read output from a session.
        pub async fn read_output(
            &self,
            session_id: &str,
            buf: &mut [u8],
        ) -> Result<usize, RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                RemoteError::SessionNotFound(session_id.into())
            })?;

            session.last_activity = Instant::now();
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
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                RemoteError::SessionNotFound(session_id.into())
            })?;

            session.cols = cols;
            session.rows = rows;
            session.pty.resize(cols, rows)
        }

        /// Close a session.
        pub async fn close(&self, session_id: &str, user: Option<&str>) -> Result<Option<u32>, RemoteError> {
            let mut sessions = self.sessions.lock().await;
            let mut session = sessions.remove(session_id).ok_or_else(|| {
                RemoteError::SessionNotFound(session_id.into())
            })?;

            let exit_code = session.pty.try_wait();
            session.pty.kill();

            self.audit.log_session_close(session_id, exit_code, user).await;

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
}

#[cfg(feature = "cli")]
pub use session_impl::{SessionConfig, SessionManager};
