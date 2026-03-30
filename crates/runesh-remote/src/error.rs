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
        RemoteError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }
}
