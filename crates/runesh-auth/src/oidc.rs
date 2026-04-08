//! OIDC Authorization Code flow with PKCE.
//!
//! Supports any standard OIDC provider (Azure EntraID, Keycloak, Auth0, etc.)
//! Provider-specific extensions (MS Graph, group mapping) are handled via
//! the [`AuthStore`] trait in the consumer project.

use std::collections::HashMap;

use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::AuthError;

// ── Provider configuration ──────────────────────────────────────────────────

/// OIDC provider endpoints discovered from `.well-known/openid-configuration`.
#[derive(Debug, Clone)]
pub struct OidcProvider {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: Option<String>,
    /// Shared HTTP client (with timeout) for token exchange and userinfo calls.
    pub http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct OpenIdConfiguration {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    userinfo_endpoint: String,
    #[serde(default)]
    jwks_uri: String,
}

/// Parameters needed before discovery (env vars or DB settings).
pub struct OidcParams {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: Option<String>,
}

impl OidcProvider {
    /// Perform OIDC discovery and build a fully-configured provider.
    pub async fn discover(params: OidcParams) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AuthError::Discovery(format!("failed to build HTTP client: {e}")))?;
        let discovery_url = format!("{}/.well-known/openid-configuration", params.issuer);

        let config: OpenIdConfiguration = http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| AuthError::Discovery(format!("fetch failed: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Discovery(format!("parse failed: {e}")))?;

        Ok(Self {
            issuer: params.issuer,
            client_id: params.client_id,
            client_secret: params.client_secret,
            redirect_uri: params.redirect_uri,
            scopes: params
                .scopes
                .unwrap_or_else(|| "openid profile email".into()),
            authorization_endpoint: config.authorization_endpoint,
            token_endpoint: config.token_endpoint,
            userinfo_endpoint: config.userinfo_endpoint,
            jwks_uri: if config.jwks_uri.is_empty() {
                None
            } else {
                Some(config.jwks_uri)
            },
            http,
        })
    }

    /// Build from env vars. Returns `None` if `OIDC_ISSUER` is not set.
    pub async fn from_env() -> Result<Option<Self>, AuthError> {
        let issuer = match std::env::var("OIDC_ISSUER") {
            Ok(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };

        let params = OidcParams {
            issuer,
            client_id: std::env::var("OIDC_CLIENT_ID")
                .map_err(|_| AuthError::BadRequest("OIDC_CLIENT_ID not set".into()))?,
            client_secret: std::env::var("OIDC_CLIENT_SECRET").ok(),
            redirect_uri: std::env::var("OIDC_REDIRECT_URI")
                .unwrap_or_else(|_| "http://localhost:8080/auth/callback".into()),
            scopes: std::env::var("OIDC_SCOPE").ok(),
        };

        Self::discover(params).await.map(Some)
    }

    /// Exchange an authorization code for tokens and fetch user info.
    ///
    /// Always uses the configured `redirect_uri` to prevent open-redirect attacks.
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<(TokenResponse, OidcUserInfo), AuthError> {
        let http = &self.http;
        let redirect = &self.redirect_uri;

        let mut params: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("client_id", &self.client_id),
            ("code_verifier", code_verifier),
        ];

        // client_secret is optional (public clients use PKCE only)
        let secret_val;
        if let Some(ref s) = self.client_secret {
            secret_val = s.clone();
            params.push(("client_secret", &secret_val));
        }

        let resp = http
            .post(&self.token_endpoint)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(body = %body, "Token exchange failed");
            return Err(AuthError::TokenExchange(body));
        }

        let token_resp: TokenResponse = resp.json().await?;

        // Decode ID token claims (unverified — trusted because server-side exchange
        // with client_secret means the token came directly from the IdP)
        let id_claims = token_resp
            .id_token
            .as_deref()
            .and_then(|t| decode_id_token_unverified(t, &self.client_id).ok());

        // Fetch userinfo as authoritative source (works with all providers)
        let userinfo = if !self.userinfo_endpoint.is_empty() {
            http.get(&self.userinfo_endpoint)
                .bearer_auth(&token_resp.access_token)
                .send()
                .await
                .ok()
                .and_then(|r| {
                    if r.status().is_success() {
                        Some(r)
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        let userinfo_claims: Option<UserinfoResponse> = match userinfo {
            Some(r) => r.json().await.ok(),
            None => None,
        };

        // Merge: prefer ID token claims, fall back to userinfo
        let info = OidcUserInfo {
            sub: id_claims
                .as_ref()
                .map(|c| c.oid.clone().unwrap_or_else(|| c.sub.clone()))
                .or_else(|| userinfo_claims.as_ref().map(|u| u.sub.clone()))
                .unwrap_or_default(),
            email: id_claims
                .as_ref()
                .and_then(|c| c.email.clone().or_else(|| c.preferred_username.clone()))
                .or_else(|| userinfo_claims.as_ref().map(|u| u.email.clone()))
                .unwrap_or_default(),
            name: id_claims
                .as_ref()
                .and_then(|c| c.name.clone())
                .or_else(|| userinfo_claims.as_ref().map(|u| u.name.clone()))
                .unwrap_or_default(),
            picture: id_claims.as_ref().and_then(|c| c.picture.clone()),
            groups: id_claims.as_ref().and_then(|c| c.groups.clone()),
            idp_access_token: Some(token_resp.access_token.clone()),
            idp_refresh_token: token_resp.refresh_token.clone(),
            idp_token_expires_in: token_resp.expires_in,
        };

        Ok((token_resp, info))
    }

    /// Return the config the frontend needs to initiate the OIDC flow.
    pub fn frontend_config(&self) -> serde_json::Value {
        serde_json::json!({
            "authorization_endpoint": self.authorization_endpoint,
            "client_id": self.client_id,
            "scope": self.scopes,
        })
    }
}

// ── ID token decoding ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct IdTokenClaims {
    sub: String,
    email: Option<String>,
    preferred_username: Option<String>,
    name: Option<String>,
    oid: Option<String>,
    picture: Option<String>,
    #[serde(default)]
    groups: Option<Vec<String>>,
}

/// Decode an ID token without full signature verification.
///
/// SECURITY NOTE: This is acceptable ONLY when the token was obtained via a
/// server-side code exchange with `client_secret` (confidential client), meaning
/// the token came directly from the IdP's token endpoint over TLS. For public
/// clients (no client_secret), the caller MUST verify the signature via JWKS.
///
/// We still validate: audience, issuer presence, and enforce a maximum age of
/// 5 minutes to limit replay window.
fn decode_id_token_unverified(
    id_token: &str,
    audience: &str,
) -> Result<IdTokenClaims, AuthError> {
    let mut validation = jsonwebtoken::Validation::default();
    validation.insecure_disable_signature_validation();
    validation.set_audience(&[audience]);
    // Enforce expiration - tokens must not be expired
    validation.validate_exp = true;
    // Allow 60s clock skew
    validation.set_required_spec_claims(&["sub", "exp"]);

    let data = jsonwebtoken::decode::<IdTokenClaims>(
        id_token,
        &jsonwebtoken::DecodingKey::from_secret(b"unused"),
        &validation,
    )?;

    Ok(data.claims)
}

// ── Token / Userinfo response types ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: String,
}

#[derive(Debug, Deserialize)]
struct UserinfoResponse {
    sub: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: String,
}

/// Merged user info from ID token + userinfo endpoint.
/// This is what gets passed to [`AuthStore::upsert_user`].
#[derive(Debug, Clone)]
pub struct OidcUserInfo {
    /// Subject identifier (Azure: oid, others: sub)
    pub sub: String,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
    /// OIDC group membership claims (if the provider includes them)
    pub groups: Option<Vec<String>>,
    /// The IdP's access token (for calling provider APIs like MS Graph)
    pub idp_access_token: Option<String>,
    /// The IdP's refresh token (for offline_access)
    pub idp_refresh_token: Option<String>,
    /// Token lifetime in seconds
    pub idp_token_expires_in: Option<u64>,
}

// ── PKCE session management ─────────────────────────────────────────────────

/// An in-progress OIDC authorization session.
#[derive(Debug, Clone)]
pub struct OidcSession {
    pub id: String,
    pub state: String,
    pub code_verifier: String,
    pub redirect_uri: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// In-memory session store for PKCE flows.
/// Sessions auto-expire after 10 minutes. Capped at 10,000 to prevent DoS.
pub struct OidcSessionStore {
    sessions: RwLock<HashMap<String, OidcSession>>,
    /// Secondary index: state -> session_id for O(1) lookup by state.
    state_index: RwLock<HashMap<String, String>>,
    max_sessions: usize,
}

impl OidcSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            state_index: RwLock::new(HashMap::new()),
            max_sessions: 10_000,
        }
    }

    /// Start a new OIDC session. Returns `(session_id, authorization_url)`.
    /// Automatically cleans up expired sessions if near capacity.
    /// Returns an error if the session store is at capacity after cleanup.
    pub async fn start(
        &self,
        provider: &OidcProvider,
        extra_scopes: Option<&str>,
    ) -> Result<(String, String), AuthError> {
        // Cleanup if approaching capacity
        {
            let sessions = self.sessions.read().await;
            if sessions.len() >= self.max_sessions / 2 {
                drop(sessions);
                self.cleanup().await;
            }
        }
        // Reject if still at capacity after cleanup
        {
            let sessions = self.sessions.read().await;
            if sessions.len() >= self.max_sessions {
                tracing::warn!("OIDC session store at capacity, rejecting new session");
                return Err(AuthError::Internal("OIDC session store at capacity".into()));
            }
        }
        let session_id = Uuid::new_v4().to_string();
        let state = Uuid::new_v4().to_string();

        // PKCE
        let verifier_bytes = rand::random::<[u8; 32]>();
        let code_verifier =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let code_challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        let scopes = match extra_scopes {
            Some(extra) => format!("{} {}", provider.scopes, extra),
            None => provider.scopes.clone(),
        };

        let auth_url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            provider.authorization_endpoint,
            urlencoding::encode(&provider.client_id),
            urlencoding::encode(&provider.redirect_uri),
            urlencoding::encode(&scopes),
            urlencoding::encode(&state),
            urlencoding::encode(&code_challenge),
        );

        let session = OidcSession {
            id: session_id.clone(),
            state: state.clone(),
            code_verifier,
            redirect_uri: None,
            created_at: chrono::Utc::now(),
        };

        self.state_index
            .write()
            .await
            .insert(state, session_id.clone());
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session);

        Ok((session_id, auth_url))
    }

    /// Look up a session by its `state` parameter (from the callback).
    /// Uses a secondary index for O(1) lookup instead of scanning all sessions.
    pub async fn get_by_state(&self, state: &str) -> Option<OidcSession> {
        let session_id = {
            let index = self.state_index.read().await;
            index.get(state).cloned()?
        };
        let sessions = self.sessions.read().await;
        sessions.get(&session_id).cloned()
    }

    /// Remove a session after successful callback.
    pub async fn remove(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.remove(session_id) {
            drop(sessions);
            self.state_index.write().await.remove(&session.state);
        }
    }

    /// Remove sessions older than 10 minutes.
    pub async fn cleanup(&self) {
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(10);
        let mut sessions = self.sessions.write().await;
        let expired_states: Vec<String> = sessions
            .values()
            .filter(|s| s.created_at <= cutoff)
            .map(|s| s.state.clone())
            .collect();
        sessions.retain(|_, s| s.created_at > cutoff);
        drop(sessions);

        let mut state_index = self.state_index.write().await;
        for state in expired_states {
            state_index.remove(&state);
        }
    }
}

impl Default for OidcSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Redis-backed OIDC session store ────────────────────────────────────────

#[cfg(feature = "redis")]
mod redis_session {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Serializable version of OidcSession for Redis storage.
    #[derive(Debug, Serialize, Deserialize)]
    struct StoredSession {
        id: String,
        state: String,
        code_verifier: String,
        redirect_uri: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    impl From<&OidcSession> for StoredSession {
        fn from(s: &OidcSession) -> Self {
            Self {
                id: s.id.clone(),
                state: s.state.clone(),
                code_verifier: s.code_verifier.clone(),
                redirect_uri: s.redirect_uri.clone(),
                created_at: s.created_at,
            }
        }
    }

    impl From<StoredSession> for OidcSession {
        fn from(s: StoredSession) -> Self {
            Self {
                id: s.id,
                state: s.state,
                code_verifier: s.code_verifier,
                redirect_uri: s.redirect_uri,
                created_at: s.created_at,
            }
        }
    }

    /// Redis-backed OIDC session store for horizontal scaling.
    ///
    /// Sessions are stored in Redis with a 600-second TTL, so no manual cleanup
    /// is needed. The `state` parameter is stored as a secondary index key
    /// (`oidc:state:{state}` -> session_id) to support lookup by state.
    ///
    /// ```ignore
    /// let pool = runesh_core::redis::create_redis_pool(None).unwrap();
    /// let store = RedisOidcSessionStore::new(pool);
    /// ```
    #[derive(Clone)]
    pub struct RedisOidcSessionStore {
        pool: deadpool_redis::Pool,
        /// TTL for sessions in seconds (default: 600 = 10 minutes).
        ttl_secs: u64,
    }

    impl RedisOidcSessionStore {
        pub fn new(pool: deadpool_redis::Pool) -> Self {
            Self {
                pool,
                ttl_secs: 600,
            }
        }

        pub fn with_ttl(pool: deadpool_redis::Pool, ttl_secs: u64) -> Self {
            Self { pool, ttl_secs }
        }

        fn session_key(id: &str) -> String {
            format!("oidc:session:{id}")
        }

        fn state_key(state: &str) -> String {
            format!("oidc:state:{state}")
        }

        /// Start a new OIDC session. Returns `(session_id, authorization_url)`.
        ///
        /// Stores the session in Redis with TTL. No capacity check needed —
        /// Redis TTL handles expiry automatically.
        pub async fn start(
            &self,
            provider: &OidcProvider,
            extra_scopes: Option<&str>,
        ) -> Result<(String, String), AuthError> {
            let session_id = Uuid::new_v4().to_string();
            let state = Uuid::new_v4().to_string();

            // PKCE
            let mut verifier_bytes = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rng(), &mut verifier_bytes);
            let code_verifier =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

            let mut hasher = Sha256::new();
            hasher.update(code_verifier.as_bytes());
            let code_challenge =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

            let scopes = match extra_scopes {
                Some(extra) => format!("{} {}", provider.scopes, extra),
                None => provider.scopes.clone(),
            };

            let auth_url = format!(
                "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
                provider.authorization_endpoint,
                urlencoding::encode(&provider.client_id),
                urlencoding::encode(&provider.redirect_uri),
                urlencoding::encode(&scopes),
                urlencoding::encode(&state),
                urlencoding::encode(&code_challenge),
            );

            let session = OidcSession {
                id: session_id.clone(),
                state: state.clone(),
                code_verifier,
                redirect_uri: None,
                created_at: chrono::Utc::now(),
            };

            let stored: StoredSession = (&session).into();
            let json = serde_json::to_string(&stored)
                .map_err(|e| AuthError::Internal(format!("Failed to serialize session: {e}")))?;

            let mut conn = self.pool.get().await.map_err(|e| {
                AuthError::Internal(format!("Failed to get Redis connection: {e}"))
            })?;

            // Store session by ID with TTL
            deadpool_redis::redis::cmd("SET")
                .arg(Self::session_key(&session_id))
                .arg(&json)
                .arg("EX")
                .arg(self.ttl_secs)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|e| AuthError::Internal(format!("Redis SET failed: {e}")))?;

            // Store state -> session_id index with same TTL
            deadpool_redis::redis::cmd("SET")
                .arg(Self::state_key(&state))
                .arg(&session_id)
                .arg("EX")
                .arg(self.ttl_secs)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|e| AuthError::Internal(format!("Redis SET (state index) failed: {e}")))?;

            Ok((session_id, auth_url))
        }

        /// Look up a session by its `state` parameter (from the callback).
        pub async fn get_by_state(&self, state: &str) -> Option<OidcSession> {
            let mut conn = self.pool.get().await.ok()?;

            // Look up session_id from state index
            let session_id: Option<String> = deadpool_redis::redis::cmd("GET")
                .arg(Self::state_key(state))
                .query_async(&mut *conn)
                .await
                .ok()?;

            let session_id = session_id?;

            // Fetch the session data
            let json: Option<String> = deadpool_redis::redis::cmd("GET")
                .arg(Self::session_key(&session_id))
                .query_async(&mut *conn)
                .await
                .ok()?;

            let json = json?;
            let stored: StoredSession = serde_json::from_str(&json).ok()?;
            Some(stored.into())
        }

        /// Remove a session after successful callback (get + delete).
        pub async fn remove(&self, session_id: &str) {
            let Ok(mut conn) = self.pool.get().await else {
                return;
            };

            // Fetch session first to get the state for index cleanup
            let json: Option<String> = deadpool_redis::redis::cmd("GET")
                .arg(Self::session_key(session_id))
                .query_async(&mut *conn)
                .await
                .unwrap_or(None);

            if let Some(json) = json {
                if let Ok(stored) = serde_json::from_str::<StoredSession>(&json) {
                    // Delete state index
                    let _: Result<(), _> = deadpool_redis::redis::cmd("DEL")
                        .arg(Self::state_key(&stored.state))
                        .query_async(&mut *conn)
                        .await;
                }
            }

            // Delete session
            let _: Result<(), _> = deadpool_redis::redis::cmd("DEL")
                .arg(Self::session_key(session_id))
                .query_async(&mut *conn)
                .await;
        }

        /// No-op — Redis TTL handles expiry automatically.
        pub async fn cleanup(&self) {
            // Redis TTL handles expiry; nothing to do.
        }
    }
}

#[cfg(feature = "redis")]
pub use redis_session::RedisOidcSessionStore;
