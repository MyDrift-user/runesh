//! JWT access token generation and validation + refresh token utilities.

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
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
    /// Expected `iss` claim. When set, both `encode_access_token` and
    /// `validate_access_token` enforce it.
    pub required_iss: Option<String>,
    /// Expected `aud` claim values. When non-empty, both `encode_access_token`
    /// and `validate_access_token` enforce it.
    pub required_aud: Vec<String>,
}

impl TokenConfig {
    /// Create a new token config with iss/aud unset.
    ///
    /// Returns [`AuthError::BadRequest`] if the secret is shorter than 32 bytes.
    /// Callers should propagate with `?`.
    pub fn new(secret: String) -> Result<Self, AuthError> {
        if secret.len() < 32 {
            return Err(AuthError::BadRequest(
                "JWT secret too short (expected >= 32 bytes for HMAC-SHA256)".into(),
            ));
        }
        Ok(Self {
            secret,
            access_token_ttl: 900,
            refresh_token_ttl: 2_592_000,
            required_iss: None,
            required_aud: Vec::new(),
        })
    }

    /// Builder: set the required issuer.
    pub fn with_issuer(mut self, iss: impl Into<String>) -> Self {
        self.required_iss = Some(iss.into());
        self
    }

    /// Builder: set the required audience values.
    pub fn with_audience<I, S>(mut self, aud: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_aud = aud.into_iter().map(Into::into).collect();
        self
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
    /// Issuer. Populated when the config has `required_iss` set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// Audience. Populated when the config has `required_aud` set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<Vec<String>>,
}

// ── Access tokens ───────────────────────────────────────────────────────────

/// Issue a signed JWT access token. Returns `(token_string, expires_in_seconds)`.
///
/// When `config.required_iss` or `config.required_aud` is set, the
/// corresponding claim is embedded in the token. Emitting a token that omits
/// a configured required claim would leave the validator permanently unable
/// to accept it, so this function returns an `Internal` error if the config
/// is internally inconsistent (empty iss/aud strings with `required_iss` set).
pub fn issue_access_token(
    config: &TokenConfig,
    user_id: &str,
    email: &str,
    name: &str,
    role: &str,
    permissions: &[String],
) -> Result<(String, i64), AuthError> {
    let now = Utc::now().timestamp();

    if let Some(iss) = &config.required_iss
        && iss.is_empty()
    {
        return Err(AuthError::Internal(
            "TokenConfig.required_iss set but empty".into(),
        ));
    }
    if config.required_aud.iter().any(|a| a.is_empty()) {
        return Err(AuthError::Internal(
            "TokenConfig.required_aud contains empty string".into(),
        ));
    }

    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        permissions: permissions.to_vec(),
        exp: now + config.access_token_ttl,
        iat: now,
        iss: config.required_iss.clone(),
        aud: if config.required_aud.is_empty() {
            None
        } else {
            Some(config.required_aud.clone())
        },
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
/// When `config` is `Some`, `iss` and `aud` are enforced against the values
/// declared in the config. `sub` and `exp` are always required.
pub fn validate_access_token(token: &str, secret: &str) -> Result<Claims, AuthError> {
    validate_access_token_with(token, secret, None)
}

/// Validate an access token with an optional [`TokenConfig`] that supplies
/// the expected `iss` / `aud` values.
pub fn validate_access_token_with(
    token: &str,
    secret: &str,
    config: Option<&TokenConfig>,
) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let mut required: Vec<&str> = vec!["sub", "exp"];
    if let Some(cfg) = config {
        if let Some(iss) = &cfg.required_iss {
            validation.set_issuer(&[iss.as_str()]);
            required.push("iss");
        }
        if !cfg.required_aud.is_empty() {
            let aud: Vec<&str> = cfg.required_aud.iter().map(|s| s.as_str()).collect();
            validation.set_audience(&aud);
            required.push("aud");
        } else {
            // jsonwebtoken defaults to requiring aud; disable when caller
            // didn't opt in, otherwise tokens without aud would fail.
            validation.validate_aud = false;
        }
    } else {
        validation.validate_aud = false;
    }
    validation.set_required_spec_claims(&required);

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
