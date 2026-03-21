//! JWT access token generation and validation + refresh token utilities.

use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::AuthError;

// ── Configuration ───────────────────────────────────────────────────────────

/// Token timing configuration.
pub struct TokenConfig {
    /// JWT signing secret.
    pub secret: String,
    /// Access token lifetime in seconds (default: 900 = 15 minutes).
    pub access_token_ttl: i64,
    /// Refresh token lifetime in seconds (default: 2592000 = 30 days).
    pub refresh_token_ttl: i64,
}

impl TokenConfig {
    pub fn new(secret: String) -> Self {
        Self {
            secret,
            access_token_ttl: 900,
            refresh_token_ttl: 2_592_000,
        }
    }
}

// ── JWT Claims ──────────────────────────────────────────────────────────────

/// Claims embedded in every JWT. Projects can extend by embedding additional
/// data in their [`AuthStore`] response rather than the JWT itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User ID (UUID string).
    pub sub: String,
    /// User email.
    pub email: String,
    /// Display name.
    pub name: String,
    /// Effective role (e.g. "admin", "user", "manager").
    pub role: String,
    /// Permission strings loaded from the project's RBAC system.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Expiration (Unix timestamp).
    pub exp: i64,
    /// Issued at (Unix timestamp).
    pub iat: i64,
}

// ── Access tokens ───────────────────────────────────────────────────────────

/// Issue a signed JWT access token. Returns `(token_string, expires_in_seconds)`.
pub fn issue_access_token(
    config: &TokenConfig,
    user_id: &str,
    email: &str,
    name: &str,
    role: &str,
    permissions: &[String],
) -> Result<(String, i64), AuthError> {
    let now = Utc::now().timestamp();

    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        permissions: permissions.to_vec(),
        exp: now + config.access_token_ttl,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.secret.as_bytes()),
    )?;

    Ok((token, config.access_token_ttl))
}

/// Validate an access token and return its claims.
pub fn validate_access_token(token: &str, secret: &str) -> Result<Claims, AuthError> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

// ── Refresh tokens ──────────────────────────────────────────────────────────

/// Generate a new random refresh token string.
pub fn generate_refresh_token() -> String {
    Uuid::new_v4().to_string() + &Uuid::new_v4().to_string()
}

/// Hash a refresh token for safe storage.
pub fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compute the refresh token expiry timestamp.
pub fn refresh_token_expiry(config: &TokenConfig) -> chrono::DateTime<Utc> {
    Utc::now() + chrono::Duration::seconds(config.refresh_token_ttl)
}

// ── Permission helpers ──────────────────────────────────────────────────────

/// Check if claims contain a specific permission. Admins always pass.
pub fn has_permission(claims: &Claims, permission: &str) -> bool {
    claims.role == "admin" || claims.permissions.contains(&permission.to_string())
}
