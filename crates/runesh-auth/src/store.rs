//! Trait that projects implement to connect auth to their database and
//! provider-specific logic.
//!
//! This is the main extensibility point. For example:
//! - HARUMI implements `on_oidc_login` to fetch MS Graph profile photos and
//!   map Azure group claims to local roles.
//! - HARUMI-NET implements a simpler version that just creates users.
//! - A project using Keycloak could map Keycloak roles differently.

use async_trait::async_trait;

use crate::error::AuthError;
use crate::oidc::OidcUserInfo;
use crate::token::Claims;

/// Information returned after a user is resolved from the store.
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// User ID (UUID string).
    pub id: String,
    pub email: String,
    pub name: String,
    /// Effective role for JWT claims.
    pub role: String,
    /// Avatar URL (if available).
    pub avatar_url: Option<String>,
    /// Permissions loaded from the project's RBAC system.
    pub permissions: Vec<String>,
}

/// Implement this trait in your project to connect the auth system to your
/// database and any provider-specific extensions.
#[async_trait]
pub trait AuthStore: Send + Sync + 'static {
    // ── Required ────────────────────────────────────────────────────────

    /// Find or create a user from OIDC claims. This is called after a
    /// successful OIDC code exchange.
    ///
    /// Use this to:
    /// - Upsert the user in your database
    /// - Store the IdP tokens (`info.idp_access_token`, `info.idp_refresh_token`)
    /// - Map OIDC groups to local roles (via `info.groups`)
    /// - Fetch provider-specific data (e.g. MS Graph profile photo)
    /// - Return the user's effective role and permissions
    async fn upsert_user(&self, info: &OidcUserInfo) -> Result<AuthUser, AuthError>;

    /// Look up a user by their ID (for the refresh token flow).
    /// Should also load current role and permissions.
    async fn get_user_by_id(&self, user_id: &str) -> Result<AuthUser, AuthError>;

    /// Store a refresh token hash with its expiry.
    async fn store_refresh_token(
        &self,
        user_id: &str,
        token_hash: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AuthError>;

    /// Validate a refresh token hash: check it exists and isn't expired,
    /// then delete it (consume). Return the `user_id`.
    async fn consume_refresh_token(&self, token_hash: &str) -> Result<String, AuthError>;

    /// Revoke all refresh tokens for a user (logout).
    async fn revoke_all_refresh_tokens(&self, user_id: &str) -> Result<(), AuthError>;

    // ── Optional ────────────────────────────────────────────────────────

    /// Optional: password-based login. Return `None` if your project doesn't
    /// support local auth. Default returns `None`.
    async fn verify_password(
        &self,
        _email: &str,
        _password: &str,
    ) -> Result<Option<AuthUser>, AuthError> {
        Ok(None)
    }

    /// Optional: called after successful login (OIDC or password) to update
    /// `last_login_at` or similar. Default is a no-op.
    async fn on_login(&self, _user_id: &str) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Helper: build a [`Claims`] struct from an [`AuthUser`] and config.
pub fn claims_from_user(user: &AuthUser, config: &crate::token::TokenConfig) -> Claims {
    let now = chrono::Utc::now().timestamp();
    Claims {
        sub: user.id.clone(),
        email: user.email.clone(),
        name: user.name.clone(),
        role: user.role.clone(),
        permissions: user.permissions.clone(),
        exp: now + config.access_token_ttl,
        iat: now,
    }
}
