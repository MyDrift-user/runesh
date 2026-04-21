//! Authentication, authorization, and consent for desktop WebSocket endpoints.
//!
//! Crate-local (no dep on `runesh-auth`) to avoid cycles. Consumers wire
//! a concrete [`DesktopAuth`] plus a [`ConsentBroker`] into the state.

use async_trait::async_trait;

/// Identity established by a successful [`DesktopAuth::authenticate`] call.
#[derive(Debug, Clone)]
pub struct Principal {
    pub subject: String,
    pub display_name: Option<String>,
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

/// Operations authorized on a desktop session.
#[derive(Debug, Clone)]
pub enum Operation {
    StartSession,
    StopSession,
    View,
    InjectInput,
    SetClipboard,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("authentication failed: {0}")]
    Unauthenticated(String),
    #[error("authorization denied: {0}")]
    Forbidden(String),
    #[error("internal auth error: {0}")]
    Internal(String),
}

#[async_trait]
pub trait DesktopAuth: Send + Sync + 'static {
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError>;
    fn authorize(&self, principal: &Principal, op: &Operation) -> Result<(), AuthError>;
}

/// Consent broker: asks the local user (out of band) whether to grant
/// input-injection rights to a remote requester.
#[async_trait]
pub trait ConsentBroker: Send + Sync + 'static {
    /// Called once per session when the remote requester asks for input.
    /// Must return `true` only after a real user interaction.
    async fn request_input_consent(&self, session_id: &str, requester: &Principal) -> bool;

    /// Optional per-direction clipboard consent. Default false.
    async fn request_clipboard_consent(&self, _session_id: &str, _requester: &Principal) -> bool {
        false
    }
}

/// The safe default: always deny input. View-only sessions.
pub struct AlwaysDeny;

#[async_trait]
impl ConsentBroker for AlwaysDeny {
    async fn request_input_consent(&self, _session_id: &str, _requester: &Principal) -> bool {
        false
    }
}

/// Loudly-named test helper that lets everything through.
pub struct AllowAllAuth;

#[async_trait]
impl DesktopAuth for AllowAllAuth {
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

/// Safe default auth that refuses every request.
pub struct DenyAllAuth;

#[async_trait]
impl DesktopAuth for DenyAllAuth {
    async fn authenticate(&self, _token: &str) -> Result<Principal, AuthError> {
        Err(AuthError::Unauthenticated(
            "no desktop auth backend configured (DenyAllAuth)".into(),
        ))
    }
    fn authorize(&self, _principal: &Principal, _op: &Operation) -> Result<(), AuthError> {
        Err(AuthError::Forbidden(
            "no desktop auth backend configured (DenyAllAuth)".into(),
        ))
    }
}
