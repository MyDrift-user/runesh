//! Inventory collection error types.

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("Collection failed: {0}")]
    Collection(String),

    #[error("Platform not supported: {0}")]
    Unsupported(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Timeout collecting {0}")]
    Timeout(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl InventoryError {
    pub fn status_code(&self) -> u16 {
        match self {
            InventoryError::Collection(_) => 500,
            InventoryError::Unsupported(_) => 501,
            InventoryError::PermissionDenied(_) => 403,
            InventoryError::Timeout(_) => 504,
            InventoryError::Serialization(_) => 500,
            InventoryError::Internal(_) => 500,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            InventoryError::Collection(_) => "collection_failed",
            InventoryError::Unsupported(_) => "unsupported",
            InventoryError::PermissionDenied(_) => "permission_denied",
            InventoryError::Timeout(_) => "timeout",
            InventoryError::Serialization(_) => "serialization_error",
            InventoryError::Internal(_) => "internal",
        }
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for InventoryError {
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

impl From<serde_json::Error> for InventoryError {
    fn from(e: serde_json::Error) -> Self {
        InventoryError::Serialization(e.to_string())
    }
}
