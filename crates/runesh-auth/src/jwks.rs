//! JWKS-backed verification for OIDC bearer tokens.
//!
//! `runesh-auth`'s default `auth_middleware` validates first-party HS256 JWTs
//! signed with `JWT_SECRET`. This module adds an alternative path for tokens
//! issued directly by an OIDC IdP (Keycloak, Azure EntraID, Auth0, …) which
//! are typically RS256 and signed by a key published at the IdP's `jwks_uri`.
//!
//! ## Usage
//!
//! Build a verifier at startup and add it as an Axum extension:
//!
//! ```ignore
//! let oidc = runesh_auth::OidcVerifier::from_env().await?;
//! let app = Router::new()
//!     // ... routes ...
//!     .layer(middleware::from_fn(auth_middleware))
//!     .layer(axum::Extension(oidc));
//! ```
//!
//! When the verifier is present, tokens whose JWT header `alg` is RS256 / ES256
//! are validated against the JWKS instead of the local `JWT_SECRET`. The
//! resulting claims are then mapped onto the existing `Claims` struct so the
//! rest of the request pipeline (handlers, RBAC checks) is identical regardless
//! of which path validated the token.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::AuthError;
use crate::token::Claims;

/// Cached JWK material for one IdP.
struct CachedKeys {
    /// `kid` -> decoding key.
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
}

/// OIDC bearer-token verifier.
///
/// Cheap to clone — internally an `Arc`. Build once at startup and add it as
/// an Axum `Extension`.
#[derive(Clone)]
pub struct OidcVerifier {
    inner: Arc<OidcVerifierInner>,
}

struct OidcVerifierInner {
    /// Expected `iss` claim. Validation rejects tokens that don't match.
    issuer: String,
    /// Optional expected `aud` claim. If `None`, audience is not enforced —
    /// useful when the IdP issues access tokens with `aud=account` (Keycloak
    /// default) and you want to accept any.
    audience: Option<String>,
    /// IdP JWKS endpoint discovered from `.well-known/openid-configuration`.
    jwks_uri: String,
    /// Shared HTTP client for JWKS fetches.
    http: reqwest::Client,
    /// Cache TTL — re-fetch the JWKS if older than this. IdP key rotation
    /// is rare, but we want to pick up new keys eventually without a restart.
    cache_ttl: Duration,
    keys: RwLock<Option<CachedKeys>>,
}

impl OidcVerifier {
    /// Build a verifier by fetching the IdP's `.well-known/openid-configuration`
    /// to discover the JWKS endpoint. Issuer should be the canonical issuer
    /// URL (no trailing slash, no `/.well-known/...` suffix).
    pub async fn discover(
        issuer: impl Into<String>,
        audience: Option<String>,
    ) -> Result<Self, AuthError> {
        let issuer = issuer.into();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AuthError::Discovery(format!("http client: {e}")))?;

        let discovery_url = format!("{issuer}/.well-known/openid-configuration");
        let cfg: DiscoveryDoc = http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| AuthError::Discovery(format!("fetch {discovery_url}: {e}")))?
            .error_for_status()
            .map_err(|e| AuthError::Discovery(format!("status {discovery_url}: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Discovery(format!("parse {discovery_url}: {e}")))?;

        Ok(Self {
            inner: Arc::new(OidcVerifierInner {
                issuer,
                audience,
                jwks_uri: cfg.jwks_uri,
                http,
                cache_ttl: Duration::from_secs(3600),
                keys: RwLock::new(None),
            }),
        })
    }

    /// Build from env vars. Returns `Ok(None)` if `OIDC_ISSUER` is unset, so
    /// callers can install OIDC verification only when configured.
    ///
    /// Reads:
    /// - `OIDC_ISSUER` — required
    /// - `OIDC_AUDIENCE` — optional, enforces `aud` claim if set
    pub async fn from_env() -> Result<Option<Self>, AuthError> {
        let Some(issuer) = std::env::var("OIDC_ISSUER").ok().filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let audience = std::env::var("OIDC_AUDIENCE")
            .ok()
            .filter(|s| !s.is_empty());
        Ok(Some(Self::discover(issuer, audience).await?))
    }

    /// Validate a bearer token against the IdP's JWKS. On success, the
    /// returned [`Claims`] is suitable for insertion into request extensions
    /// — `role` defaults to `"user"` since OIDC doesn't carry first-party
    /// RBAC; consumers should map IdP roles/groups via [`AuthStore`].
    pub async fn validate(&self, token: &str) -> Result<Claims, AuthError> {
        let header = decode_header(token).map_err(AuthError::Jwt)?;
        let alg = header.alg;
        if !is_asymmetric(alg) {
            return Err(AuthError::TokenInvalid(format!(
                "OIDC verifier rejected algorithm {alg:?} (expected RS*/ES*/PS*)"
            )));
        }
        let Some(kid) = header.kid else {
            return Err(AuthError::TokenInvalid("missing kid in JWT header".into()));
        };

        let key = self.lookup_key(&kid).await?;

        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&self.inner.issuer]);
        if let Some(aud) = &self.inner.audience {
            validation.set_audience(&[aud]);
        } else {
            validation.validate_aud = false;
        }
        validation.validate_exp = true;
        validation.set_required_spec_claims(&["exp", "iss"]);

        let data = decode::<OidcClaims>(token, &key, &validation).map_err(AuthError::Jwt)?;
        Ok(data.claims.into_runesh_claims())
    }

    /// Look up a key by `kid`. Refreshes the JWKS if the key isn't in the
    /// cache (handles IdP key rotation transparently) or if the cache is
    /// older than `cache_ttl`.
    async fn lookup_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        // Fast path: read lock, key is cached and fresh.
        {
            let guard = self.inner.keys.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < self.inner.cache_ttl {
                    if let Some(key) = cached.keys.get(kid) {
                        return Ok(key.clone());
                    }
                }
            }
        }
        // Slow path: refresh and retry.
        self.refresh_keys().await?;
        let guard = self.inner.keys.read().await;
        guard
            .as_ref()
            .and_then(|c| c.keys.get(kid).cloned())
            .ok_or_else(|| AuthError::TokenInvalid(format!("kid {kid} not found in JWKS")))
    }

    async fn refresh_keys(&self) -> Result<(), AuthError> {
        let jwks: JwkSet = self
            .inner
            .http
            .get(&self.inner.jwks_uri)
            .send()
            .await
            .map_err(|e| AuthError::Discovery(format!("fetch jwks: {e}")))?
            .error_for_status()
            .map_err(|e| AuthError::Discovery(format!("jwks status: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Discovery(format!("parse jwks: {e}")))?;

        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            let kid = jwk.kid.clone();
            match decoding_key_from_jwk(&jwk) {
                Ok(key) => {
                    keys.insert(kid, key);
                }
                Err(e) => {
                    tracing::warn!(kid = %jwk.kid, kty = %jwk.kty, alg = ?jwk.alg, error = %e, "skipping unsupported JWK");
                }
            }
        }

        let mut guard = self.inner.keys.write().await;
        *guard = Some(CachedKeys {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }
}

impl std::fmt::Debug for OidcVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcVerifier")
            .field("issuer", &self.inner.issuer)
            .field("audience", &self.inner.audience)
            .field("jwks_uri", &self.inner.jwks_uri)
            .finish()
    }
}

fn is_asymmetric(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::EdDSA
    )
}

fn decoding_key_from_jwk(jwk: &Jwk) -> Result<DecodingKey, AuthError> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk
                .n
                .as_deref()
                .ok_or_else(|| AuthError::TokenInvalid("RSA jwk missing n".into()))?;
            let e = jwk
                .e
                .as_deref()
                .ok_or_else(|| AuthError::TokenInvalid("RSA jwk missing e".into()))?;
            DecodingKey::from_rsa_components(n, e).map_err(AuthError::Jwt)
        }
        "EC" => {
            let x = jwk
                .x
                .as_deref()
                .ok_or_else(|| AuthError::TokenInvalid("EC jwk missing x".into()))?;
            let y = jwk
                .y
                .as_deref()
                .ok_or_else(|| AuthError::TokenInvalid("EC jwk missing y".into()))?;
            DecodingKey::from_ec_components(x, y).map_err(AuthError::Jwt)
        }
        "OKP" => {
            let x = jwk
                .x
                .as_deref()
                .ok_or_else(|| AuthError::TokenInvalid("OKP jwk missing x".into()))?;
            DecodingKey::from_ed_components(x).map_err(AuthError::Jwt)
        }
        other => Err(AuthError::TokenInvalid(format!(
            "unsupported JWK kty: {other}"
        ))),
    }
}

// ── Wire types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DiscoveryDoc {
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    #[serde(default)]
    kid: String,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    // RSA
    n: Option<String>,
    e: Option<String>,
    // EC / OKP
    x: Option<String>,
    y: Option<String>,
}

/// Standard OIDC ID-token / access-token claim shape, lenient enough to
/// accept Keycloak, Azure EntraID, Auth0, and Google.
#[derive(Debug, Deserialize)]
struct OidcClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
    #[serde(default)]
    realm_access: Option<RealmAccess>,
}

#[derive(Debug, Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

impl OidcClaims {
    fn into_runesh_claims(self) -> Claims {
        // Keycloak-style: pick the first non-default realm role as the role,
        // fall back to "user". Other IdPs without realm_access just get "user".
        let role = self
            .realm_access
            .as_ref()
            .and_then(|ra| {
                ra.roles
                    .iter()
                    .find(|r| {
                        !r.starts_with("default-roles-")
                            && !r.starts_with("offline_")
                            && !r.starts_with("uma_")
                    })
                    .cloned()
            })
            .unwrap_or_else(|| "user".to_string());

        let permissions = self.realm_access.map(|ra| ra.roles).unwrap_or_default();

        Claims {
            sub: self.sub,
            email: self.email.unwrap_or_default(),
            name: self
                .name
                .or(self.preferred_username)
                .unwrap_or_else(|| "user".into()),
            role,
            permissions,
            exp: self.exp,
            iat: self.iat.unwrap_or(0),
        }
    }
}
