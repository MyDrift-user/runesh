//! JWT access token generation and validation + refresh token utilities.

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AuthError;

// ── Configuration ───────────────────────────────────────────────────────────

/// Token timing configuration.
pub struct TokenConfig {
    /// JWT signing secret (must be at least 32 bytes).
    pub secret: String,
    /// Access token lifetime in seconds (default: 900 = 15 minutes).
    pub access_token_ttl: i64,
    /// Refresh token lifetime in seconds (default: 2592000 = 30 days).
    pub refresh_token_ttl: i64,
}

impl TokenConfig {
    /// Create a new token config. Panics if secret is shorter than 32 bytes.
    pub fn new(secret: String) -> Self {
        assert!(
            secret.len() >= 32,
            "JWT secret must be at least 32 bytes for HMAC-SHA256 security"
        );
        Self {
            secret,
            access_token_ttl: 900,
            refresh_token_ttl: 2_592_000,
        }
    }
}

// ── JWT Claims ──────────────────────────────────────────────────────────────

/// Claims embedded in every JWT.
///
/// Only `sub`, `role`, and `exp` are load-bearing for the security boundary.
/// `email`, `name`, `permissions`, and `iat` default to empty when missing
/// so this struct can deserialize tokens issued by any consumer project,
/// regardless of which optional human-facing fields they include.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Claims {
    /// User ID (UUID string).
    pub sub: String,
    /// User email. Optional, defaults to empty when the issuing project
    /// doesn't embed email in tokens.
    #[serde(default)]
    pub email: String,
    /// Display name. Optional, defaults to empty when the issuing project
    /// doesn't embed a display name. Falls back to `username` for projects
    /// that use that field instead.
    #[serde(default, alias = "username")]
    pub name: String,
    /// Effective role (e.g. "admin", "user", "manager").
    pub role: String,
    /// Permission strings loaded from the project's RBAC system. Optional
    /// for projects that don't ship a permissions list.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Expiration (Unix timestamp).
    pub exp: i64,
    /// Issued at (Unix timestamp). Optional for tokens that omit it.
    #[serde(default)]
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

    // Explicitly use HS256 to prevent algorithm confusion attacks
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(config.secret.as_bytes()),
    )?;

    Ok((token, config.access_token_ttl))
}

/// Validate an access token and return its claims.
///
/// Pinned to HS256 to prevent algorithm confusion attacks. Tokens signed
/// with any other algorithm are rejected here; consumers that want to
/// accept asymmetric IdP tokens should layer the [`crate::OidcVerifier`]
/// in addition.
///
/// `sub` and `exp` are required. `iat` is recommended but not enforced
/// because not every issuing project embeds it.
pub fn validate_access_token(token: &str, secret: &str) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_required_spec_claims(&["sub", "exp"]);

    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims)
}

// ── Refresh tokens ──────────────────────────────────────────────────────────

/// Generate a cryptographically random 256-bit refresh token.
pub fn generate_refresh_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

/// Hash a refresh token for safe storage (SHA-256).
pub fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time comparison of a refresh token against a stored hash.
/// Use this instead of `==` to prevent timing attacks.
pub fn verify_refresh_token(token: &str, stored_hash: &str) -> bool {
    let computed = hash_refresh_token(token);
    if computed.len() != stored_hash.len() {
        return false;
    }
    // Constant-time comparison
    computed
        .as_bytes()
        .iter()
        .zip(stored_hash.as_bytes().iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
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
