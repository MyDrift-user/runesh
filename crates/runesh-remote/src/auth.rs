//! Authentication and authorization for the remote WebSocket endpoints.
//!
//! This module is crate-local (no dependency on `runesh-auth`) to avoid a
//! cycle. Consumers pass an implementation of [`RemoteAuth`] into
//! [`crate::RemoteState`] — typically a thin wrapper around their session
//! store, OIDC middleware, etc.

use async_trait::async_trait;

/// Identity established by a successful [`RemoteAuth::authenticate`] call.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Stable identifier (e.g. user id, service account id).
    pub subject: String,
    /// Optional display name for audit logs.
    pub display_name: Option<String>,
    /// Opaque roles carried across authz checks.
    pub roles: Vec<String>,
}

impl Principal {
    pub fn anonymous() -> Self {
        Self {
            subject: "anonymous".into(),
            display_name: None,
            roles: Vec::new(),
        }
    }
}

/// The operation being authorized. Keep this enum exhaustive and narrow;
/// consumer code should not need to pattern-match on free-form strings.
#[derive(Debug, Clone)]
pub enum Operation {
    FsRead,
    FsWrite,
    FsDelete,
    FsList,
    CliOpen,
    CliInput,
    CliResize,
    CliClose,
    Upload,
    Download,
}

/// Errors from the auth layer.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("authentication failed: {0}")]
    Unauthenticated(String),
    #[error("authorization denied: {0}")]
    Forbidden(String),
    #[error("internal auth error: {0}")]
    Internal(String),
}

/// Pluggable auth backend used by the WebSocket handlers.
///
/// Implementations must be cheap to `clone` through `Arc<dyn RemoteAuth>`.
#[async_trait]
pub trait RemoteAuth: Send + Sync + 'static {
    /// Exchange a client-supplied bearer token for a [`Principal`].
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError>;

    /// Check whether `principal` is allowed to perform `op`.
    fn authorize(&self, principal: &Principal, op: &Operation) -> Result<(), AuthError>;
}

/// Allow every request from any token. Only available under `cfg(test)` or
/// with the explicit `insecure-test-auth` cargo feature, so it cannot be
/// wired up by accident from a normal consumer dependency. Never enable
/// this feature on a shipped binary.
#[cfg(any(test, feature = "insecure-test-auth"))]
pub struct InsecureAllowAllAuth;

#[cfg(any(test, feature = "insecure-test-auth"))]
#[async_trait]
impl RemoteAuth for InsecureAllowAllAuth {
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        Ok(Principal {
            subject: format!("allow-all:{}", &token[..token.len().min(8)]),
            display_name: None,
            roles: vec!["allow_all".into()],
        })
    }

    fn authorize(&self, _principal: &Principal, _op: &Operation) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Reject everything. This is the safe default when a consumer forgets to
/// wire up a real auth backend.
pub struct DenyAllAuth;

#[async_trait]
impl RemoteAuth for DenyAllAuth {
    async fn authenticate(&self, _token: &str) -> Result<Principal, AuthError> {
        Err(AuthError::Unauthenticated(
            "no auth backend configured (DenyAllAuth is the safe default)".into(),
        ))
    }

    fn authorize(&self, _principal: &Principal, _op: &Operation) -> Result<(), AuthError> {
        Err(AuthError::Forbidden(
            "no auth backend configured (DenyAllAuth is the safe default)".into(),
        ))
    }
}
