//! Desktop sharing error types.

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("Screen capture failed: {0}")]
    Capture(String),

    #[error("Encoding failed: {0}")]
    Encoding(String),

    #[error("Input injection failed: {0}")]
    Input(String),

    #[error("Display not found: {0}")]
    DisplayNotFound(u32),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Platform not supported: {0}")]
    Unsupported(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Max sessions reached")]
    MaxSessions,

    #[error("Internal error: {0}")]
    Internal(String),

    /// The calling process isn't attached to an interactive user
    /// session but the chosen capture backend requires one.
    /// Triggered by `IDXGIOutput1::DuplicateOutput` returning
    /// `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE` or `E_ACCESSDENIED`
    /// from a Windows service running in Session 0, and by
    /// macOS `ScreenCaptureKit` when TCC denies capture.
    ///
    /// Callers that want to keep working regardless should retry
    /// via [`crate::session_helper::spawn_in_active_user_session`],
    /// which runs the capture inside a helper process spawned with
    /// the logged-in user's token.
    #[error("capture requires an interactive user session")]
    RequiresInteractiveSession,
}

impl DesktopError {
    pub fn status_code(&self) -> u16 {
        match self {
            DesktopError::Capture(_) => 500,
            DesktopError::Encoding(_) => 500,
            DesktopError::Input(_) => 500,
            DesktopError::DisplayNotFound(_) => 404,
            DesktopError::SessionNotFound(_) => 404,
            DesktopError::Unsupported(_) => 501,
            DesktopError::PermissionDenied(_) => 403,
            DesktopError::MaxSessions => 429,
            DesktopError::Internal(_) => 500,
            DesktopError::RequiresInteractiveSession => 409,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            DesktopError::Capture(_) => "capture_failed",
            DesktopError::Encoding(_) => "encoding_failed",
            DesktopError::Input(_) => "input_failed",
            DesktopError::DisplayNotFound(_) => "display_not_found",
            DesktopError::SessionNotFound(_) => "session_not_found",
            DesktopError::Unsupported(_) => "unsupported",
            DesktopError::PermissionDenied(_) => "permission_denied",
            DesktopError::MaxSessions => "max_sessions",
            DesktopError::Internal(_) => "internal",
            DesktopError::RequiresInteractiveSession => "requires_interactive_session",
        }
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for DesktopError {
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

impl From<std::io::Error> for DesktopError {
    fn from(e: std::io::Error) -> Self {
        DesktopError::Internal(e.to_string())
    }
}

impl From<image::ImageError> for DesktopError {
    fn from(e: image::ImageError) -> Self {
        DesktopError::Encoding(e.to_string())
    }
}
