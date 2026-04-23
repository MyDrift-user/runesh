//! Remote service error types.

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("Path not found: {0}")]
    NotFound(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Path traversal blocked: {0}")]
    PathTraversal(String),

    #[error("Operation not allowed: {0}")]
    NotAllowed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Max sessions reached")]
    MaxSessions,

    #[error("Session timeout")]
    SessionTimeout,

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),

    /// A specific Win32 step of the PTY-as-user spawn failed. Keeps
    /// the stage and the raw OS error around so callers can diagnose
    /// without parsing error strings.
    #[error("pty_as_user/{stage}: {source}")]
    Pty {
        stage: PtyStage,
        #[source]
        source: std::io::Error,
    },
}

/// Stages of the Windows PTY-as-user spawn, reported via
/// [`RemoteError::Pty`] so callers can branch on which step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyStage {
    /// `WTSGetActiveConsoleSessionId` returned no console session.
    NoActiveConsoleSession,
    /// `WTSQueryUserToken` on the active session id failed.
    QueryUserToken,
    /// `LogonUserW` on explicit credentials failed.
    LogonUser,
    /// `CreatePipe` for the ConPTY stdin or stdout handle failed.
    CreatePipe,
    /// `CreatePseudoConsole` failed.
    CreatePseudoConsole,
    /// `InitializeProcThreadAttributeList` failed.
    InitializeProcThreadAttributeList,
    /// `UpdateProcThreadAttribute` failed.
    UpdateProcThreadAttribute,
    /// `CreateEnvironmentBlock` failed.
    CreateEnvironmentBlock,
    /// `CreateProcessAsUserW` failed.
    CreateProcessAsUser,
    /// `ReadFile` on the parent-side stdout pipe failed.
    ReadFile,
    /// `WriteFile` on the parent-side stdin pipe failed.
    WriteFile,
    /// `ResizePseudoConsole` failed.
    ResizePseudoConsole,
}

impl std::fmt::Display for PtyStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::NoActiveConsoleSession => "no_active_console_session",
            Self::QueryUserToken => "query_user_token",
            Self::LogonUser => "logon_user",
            Self::CreatePipe => "create_pipe",
            Self::CreatePseudoConsole => "create_pseudo_console",
            Self::InitializeProcThreadAttributeList => "initialize_proc_thread_attribute_list",
            Self::UpdateProcThreadAttribute => "update_proc_thread_attribute",
            Self::CreateEnvironmentBlock => "create_environment_block",
            Self::CreateProcessAsUser => "create_process_as_user",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ResizePseudoConsole => "resize_pseudo_console",
        };
        f.write_str(s)
    }
}

impl RemoteError {
    pub fn status_code(&self) -> u16 {
        match self {
            RemoteError::NotFound(_) => 404,
            RemoteError::AccessDenied(_) => 403,
            RemoteError::PathTraversal(_) => 403,
            RemoteError::NotAllowed(_) => 403,
            RemoteError::Io(_) => 500,
            RemoteError::SessionNotFound(_) => 404,
            RemoteError::MaxSessions => 429,
            RemoteError::SessionTimeout => 408,
            RemoteError::BadRequest(_) => 400,
            RemoteError::Serialization(_) => 400,
            RemoteError::Internal(_) => 500,
            RemoteError::Pty { .. } => 500,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            RemoteError::NotFound(_) => "not_found",
            RemoteError::AccessDenied(_) => "access_denied",
            RemoteError::PathTraversal(_) => "path_traversal",
            RemoteError::NotAllowed(_) => "not_allowed",
            RemoteError::Io(_) => "io_error",
            RemoteError::SessionNotFound(_) => "session_not_found",
            RemoteError::MaxSessions => "max_sessions",
            RemoteError::SessionTimeout => "session_timeout",
            RemoteError::BadRequest(_) => "bad_request",
            RemoteError::Serialization(_) => "serialization_error",
            RemoteError::Internal(_) => "internal",
            RemoteError::Pty { .. } => "pty",
        }
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for RemoteError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::json!({
            "error": self.to_string(),
            "code": self.error_code(),
        });
        (status, axum::Json(body)).into_response()
    }
}

impl From<serde_json::Error> for RemoteError {
    fn from(e: serde_json::Error) -> Self {
        RemoteError::Serialization(e.to_string())
    }
}

impl From<walkdir::Error> for RemoteError {
    fn from(e: walkdir::Error) -> Self {
        RemoteError::Io(std::io::Error::other(e.to_string()))
    }
}
