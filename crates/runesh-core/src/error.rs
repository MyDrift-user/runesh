//! Shared application error type with HTTP status mapping.
//!
//! Works with both Axum (via `IntoResponse`) and can be adapted for Actix.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub code: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status_code(&self) -> u16 {
        match self {
            AppError::NotFound(_) => 404,
            AppError::Unauthorized => 401,
            AppError::Forbidden(_) => 403,
            AppError::BadRequest(_) => 400,
            AppError::Internal(_) => 500,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "not_found",
            AppError::Unauthorized => "unauthorized",
            AppError::Forbidden(_) => "forbidden",
            AppError::BadRequest(_) => "bad_request",
            AppError::Internal(_) => "internal",
        }
    }

    pub fn to_body(&self) -> ErrorBody {
        let message = match self {
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                "Internal server error".to_string()
            }
            other => other.to_string(),
        };
        ErrorBody {
            error: message,
            code: self.error_code().to_string(),
        }
    }
}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(self.to_body())).into_response()
    }
}

#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound("Resource not found".into()),
            _ => AppError::Internal(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::BadRequest(e.to_string())
    }
}
