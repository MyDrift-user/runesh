//! Admin authentication for the WinGet REST source.
//!
//! Read endpoints (`/information`, `/manifestSearch`, `/packageManifests/{id}`)
//! are open but should be rate-limited at the edge. Admin endpoints (package
//! upsert, import, delete) must require an authenticated caller.

use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use subtle_compare::constant_time_eq;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing or malformed authorization header")]
    MissingToken,
    #[error("invalid token")]
    InvalidToken,
    #[error("auth backend error: {0}")]
    Backend(String),
}

/// Authenticate the `Authorization: Bearer <token>` header for admin calls.
#[async_trait]
pub trait AdminAuth: Send + Sync {
    async fn authenticate(&self, token: &str) -> Result<(), AuthError>;
}

/// Simple static-token implementation sourced from a single admin token.
#[derive(Debug, Clone)]
pub struct StaticTokenAuth {
    expected: String,
}

impl StaticTokenAuth {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
        }
    }
}

#[async_trait]
impl AdminAuth for StaticTokenAuth {
    async fn authenticate(&self, token: &str) -> Result<(), AuthError> {
        if constant_time_eq(token.as_bytes(), self.expected.as_bytes()) {
            Ok(())
        } else {
            Err(AuthError::InvalidToken)
        }
    }
}

/// Read the admin token from `WINGET_ADMIN_TOKEN`. Callers that mount admin
/// endpoints should panic at startup if this returns `None` so an operator
/// cannot accidentally expose unauthenticated admin routes.
pub fn admin_token_from_env() -> Option<String> {
    env::var("WINGET_ADMIN_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Shared handle used by handlers.
pub type SharedAdminAuth = Arc<dyn AdminAuth>;

/// Constant-time byte comparison implemented locally so we don't pull in
/// `subtle` purely for this one call.
mod subtle_compare {
    pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut diff = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_token_matches() {
        let auth = StaticTokenAuth::new("s3cret");
        assert!(auth.authenticate("s3cret").await.is_ok());
        assert!(matches!(
            auth.authenticate("nope").await,
            Err(AuthError::InvalidToken)
        ));
        assert!(matches!(
            auth.authenticate("").await,
            Err(AuthError::InvalidToken)
        ));
    }
}
