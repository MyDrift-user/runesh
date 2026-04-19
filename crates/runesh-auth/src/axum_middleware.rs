//! Axum middleware for JWT authentication.

use axum::{
    Json,
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::token::{Claims, validate_access_token};

/// JWT secret wrapper for Axum extension injection.
#[derive(Clone)]
pub struct JwtSecret(pub String);

/// Paths that are exempt from authentication.
/// Add your project's exempt paths when building the middleware layer.
#[derive(Clone)]
pub struct AuthExemptPaths(pub Vec<String>);

impl Default for AuthExemptPaths {
    fn default() -> Self {
        // NOTE: /ws/ is NOT exempt by default -- WebSocket auth should be
        // handled by validating the token in the first message or query param
        // at the handler level, not by skipping middleware.
        Self(vec!["/auth/".into(), "/health".into()])
    }
}

/// Pluggable verifier for `X-Api-Key` headers.
///
/// Consumer projects implement this against their own user/key store
/// (typically a database lookup) and install it as an Axum extension. The
/// global [`auth_middleware`] will call into it whenever it sees an
/// `X-Api-Key` header, BEFORE falling through to JWT/cookie validation.
///
/// On success, return the [`Claims`] that should be attached to request
/// extensions for downstream handlers. The `sub`, `role`, and `exp` fields
/// are load-bearing; other fields can be defaulted.
///
/// ## Example
///
/// ```ignore
/// use std::sync::Arc;
/// use runesh_auth::{ApiKeyVerifier, ApiKeyVerifierExt, Claims};
///
/// struct PgApiKeyStore { pool: sqlx::PgPool }
///
/// #[async_trait::async_trait]
/// impl ApiKeyVerifier for PgApiKeyStore {
///     async fn verify(&self, api_key: &str) -> Option<Claims> {
///         let row: Option<(uuid::Uuid, String)> = sqlx::query_as(
///             "SELECT id, role FROM users WHERE api_key = $1"
///         ).bind(api_key).fetch_optional(&self.pool).await.ok().flatten();
///         row.map(|(id, role)| Claims {
///             sub: id.to_string(), role, exp: i64::MAX,
///             ..Default::default()
///         })
///     }
/// }
///
/// let verifier = Arc::new(PgApiKeyStore { pool });
/// let app = Router::new()
///     .layer(middleware::from_fn(auth_middleware))
///     .layer(axum::Extension(ApiKeyVerifierExt::new(verifier)));
/// ```
#[async_trait::async_trait]
pub trait ApiKeyVerifier: Send + Sync + 'static {
    async fn verify(&self, api_key: &str) -> Option<Claims>;
}

/// Type-erased Axum extension wrapper for an [`ApiKeyVerifier`].
#[derive(Clone)]
pub struct ApiKeyVerifierExt(pub std::sync::Arc<dyn ApiKeyVerifier>);

impl ApiKeyVerifierExt {
    pub fn new<V: ApiKeyVerifier>(verifier: std::sync::Arc<V>) -> Self {
        Self(verifier)
    }
}

/// Axum middleware that validates Bearer tokens and injects [`Claims`]
/// into request extensions.
///
/// # Setup
///
/// ```ignore
/// use axum::middleware;
/// use runesh_auth::axum_middleware::{auth_middleware, JwtSecret, AuthExemptPaths};
///
/// let app = Router::new()
///     .route("/api/v1/things", get(handler))
///     .layer(middleware::from_fn(auth_middleware))
///     .layer(Extension(JwtSecret("my-secret".into())))
///     .layer(Extension(AuthExemptPaths::default()));
/// ```
///
/// # Auth modes
///
/// - **Exempt paths**: requests pass through without a token.
/// - **Soft auth paths** (`/auth/*`): token is validated if present but not required.
/// - **All other paths**: valid Bearer token required, returns 401 otherwise.
pub async fn auth_middleware(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Static-asset bypass is STRICTLY limited to known static-asset path prefixes.
    // SECURITY: Earlier versions matched path SUFFIXES (e.g. `.json`, `.html`) which
    // let attackers reach API routes by crafting `/api/admin/users.json`. The new
    // rule: any path under `/api/` ALWAYS goes through full authentication, and
    // static-asset bypass only applies to well-known framework asset directories.
    let is_api = path.starts_with("/api/");
    let is_static_asset = !is_api
        && (path.starts_with("/_next/")
            || path.starts_with("/static/")
            || path.starts_with("/assets/")
            || path == "/favicon.ico"
            || path == "/robots.txt"
            || path == "/sitemap.xml"
            || path == "/manifest.webmanifest");

    if is_static_asset || method == axum::http::Method::OPTIONS {
        return next.run(req).await;
    }

    // Check exempt paths
    let exempt = req
        .extensions()
        .get::<AuthExemptPaths>()
        .cloned()
        .unwrap_or_default();

    let is_exempt = path == "/"
        || exempt.0.iter().any(|p| {
            if p.ends_with('/') {
                path.starts_with(p)
            } else {
                path == *p || path.starts_with(&format!("{}/", p))
            }
        });

    if is_exempt {
        // Soft auth: validate token if present, but don't require it
        return soft_auth(req, next).await;
    }

    // Non-API paths pass through
    if !path.starts_with("/api/") {
        return soft_auth(req, next).await;
    }

    // ── X-Api-Key path ────────────────────────────────────────────────
    // If the request carries an X-Api-Key header AND a verifier is
    // installed, validate via the verifier and short-circuit. This is
    // the SECOND auth mode used by API clients (CLIs, scripts, server-
    // to-server). Falls through to JWT/cookie validation if either the
    // header is absent or no verifier is installed.
    if let Some(api_key) = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        if let Some(verifier_ext) = req.extensions().get::<crate::ApiKeyVerifierExt>().cloned() {
            return match verifier_ext.0.verify(&api_key).await {
                Some(claims) => {
                    let mut req = req;
                    req.extensions_mut().insert(claims);
                    next.run(req).await
                }
                None => (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "invalid API key"})),
                )
                    .into_response(),
            };
        }
    }

    // Extract JWT secret
    let secret = match req.extensions().get::<JwtSecret>() {
        Some(s) => s.0.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "auth not configured"})),
            )
                .into_response();
        }
    };

    // Extract token: try Authorization header first (API clients), then cookie (sessions)
    let cookie_str = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let bearer_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .filter(|h| h.starts_with("Bearer "))
        .map(|h| h[7..].to_string());

    // Try both cookie name formats (with and without __Host- prefix)
    let cookie_token = cookie_str
        .split(';')
        .find_map(|c| {
            let c = c.trim();
            c.strip_prefix("__Host-access=")
                .or_else(|| c.strip_prefix("access="))
        })
        .map(|s| s.to_string());

    let is_cookie_auth = bearer_token.is_none() && cookie_token.is_some();

    let token = match bearer_token.or(cookie_token) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "not authenticated"})),
            )
                .into_response();
        }
    };

    // CSRF check for state-changing methods when using cookie auth
    if is_cookie_auth {
        let method = req.method().clone();
        if matches!(
            method,
            axum::http::Method::POST
                | axum::http::Method::PUT
                | axum::http::Method::PATCH
                | axum::http::Method::DELETE
        ) {
            // Verify X-CSRF-Token header matches CSRF cookie
            let csrf_cookie = cookie_str.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("__Host-csrf=")
                    .or_else(|| c.strip_prefix("csrf="))
            });
            let csrf_header = req
                .headers()
                .get("x-csrf-token")
                .and_then(|v| v.to_str().ok());

            let csrf_valid = match (csrf_cookie, csrf_header) {
                (Some(c), Some(h)) if !c.is_empty() && c.len() == h.len() => {
                    c.as_bytes()
                        .iter()
                        .zip(h.as_bytes().iter())
                        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                        == 0
                }
                _ => false,
            };

            if !csrf_valid {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "invalid CSRF token"})),
                )
                    .into_response();
            }
        }
    }

    // Two acceptable token formats:
    //   1. First-party HS256 JWT signed with JwtSecret (default).
    //   2. OIDC bearer signed by an IdP, validated against its JWKS — only
    //      when an `OidcVerifier` extension is installed.
    //
    // We pick the path by looking at the JWT header `alg`. HS* always uses the
    // local secret; RS*/ES*/PS*/EdDSA always uses OIDC. If OIDC is requested
    // but no verifier is installed, we fail closed with 401 rather than
    // accidentally accepting an unsigned token.
    let alg_is_asymmetric = jsonwebtoken::decode_header(&token)
        .map(|h| {
            matches!(
                h.alg,
                jsonwebtoken::Algorithm::RS256
                    | jsonwebtoken::Algorithm::RS384
                    | jsonwebtoken::Algorithm::RS512
                    | jsonwebtoken::Algorithm::ES256
                    | jsonwebtoken::Algorithm::ES384
                    | jsonwebtoken::Algorithm::PS256
                    | jsonwebtoken::Algorithm::PS384
                    | jsonwebtoken::Algorithm::PS512
                    | jsonwebtoken::Algorithm::EdDSA
            )
        })
        .unwrap_or(false);

    if alg_is_asymmetric {
        let verifier = req.extensions().get::<crate::OidcVerifier>().cloned();
        let Some(verifier) = verifier else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "OIDC bearer not accepted (no verifier configured)"})),
            )
                .into_response();
        };
        return match verifier.validate(&token).await {
            Ok(claims) => {
                let mut req = req;
                req.extensions_mut().insert(claims);
                next.run(req).await
            }
            Err(e) => {
                tracing::debug!(error = %e, "OIDC bearer validation failed");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "invalid or expired OIDC token"})),
                )
                    .into_response()
            }
        };
    }

    match validate_access_token(&token, &secret) {
        Ok(claims) => {
            let mut req = req;
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid or expired token"})),
        )
            .into_response(),
    }
}

/// Validate token if present, but allow the request through either way.
async fn soft_auth(req: Request<Body>, next: Next) -> Response {
    let secret = req.extensions().get::<JwtSecret>().map(|s| s.0.clone());

    if let Some(secret) = secret {
        if let Some(auth_header) = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            if auth_header.starts_with("Bearer ") {
                if let Ok(claims) = validate_access_token(&auth_header[7..], &secret) {
                    let mut req = req;
                    req.extensions_mut().insert(claims);
                    return next.run(req).await;
                }
            }
        }
    }

    next.run(req).await
}

// ── Extractor helpers ───────────────────────────────────────────────────────

/// Extract claims from request extensions. Use in Axum handlers:
///
/// ```ignore
/// async fn handler(Extension(claims): Extension<Claims>) -> impl IntoResponse { ... }
/// ```
///
/// Or use this helper for more descriptive errors:
///
/// ```ignore
/// let claims = get_claims(&req)?;
/// ```
pub fn get_claims(
    extensions: &axum::http::Extensions,
) -> Result<Claims, (StatusCode, Json<serde_json::Value>)> {
    extensions.get::<Claims>().cloned().ok_or((
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "not authenticated"})),
    ))
}

/// Check if the caller has a specific permission.
pub fn require_permission(
    claims: &Claims,
    permission: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if crate::token::has_permission(claims, permission) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "insufficient permissions"})),
        ))
    }
}
