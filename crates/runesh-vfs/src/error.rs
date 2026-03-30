//! Virtual filesystem error types.

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("File not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Read-only filesystem")]
    ReadOnly,

    #[error("Already mounted: {0}")]
    AlreadyMounted(String),

    #[error("Not mounted: {0}")]
    NotMounted(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Platform error: {0}")]
    Platform(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl VfsError {
    pub fn status_code(&self) -> u16 {
        match self {
            VfsError::NotFound(_) => 404,
            VfsError::PermissionDenied(_) => 403,
            VfsError::ReadOnly => 403,
            VfsError::AlreadyMounted(_) => 409,
            VfsError::NotMounted(_) => 404,
            VfsError::Provider(_) => 502,
            VfsError::Cache(_) => 500,
            VfsError::Io(_) => 500,
            VfsError::Platform(_) => 500,
            VfsError::Config(_) => 400,
            VfsError::Internal(_) => 500,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            VfsError::NotFound(_) => "not_found",
            VfsError::PermissionDenied(_) => "permission_denied",
            VfsError::ReadOnly => "read_only",
            VfsError::AlreadyMounted(_) => "already_mounted",
            VfsError::NotMounted(_) => "not_mounted",
            VfsError::Provider(_) => "provider_error",
            VfsError::Cache(_) => "cache_error",
            VfsError::Io(_) => "io_error",
            VfsError::Platform(_) => "platform_error",
            VfsError::Config(_) => "config_error",
            VfsError::Internal(_) => "internal",
        }
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for VfsError {
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
