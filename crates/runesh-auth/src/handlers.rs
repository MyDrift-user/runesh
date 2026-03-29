//! Ready-to-mount Axum handlers for cookie-based OIDC auth.
//!
//! Mount these in your router:
//! ```ignore
//! use runesh_auth::handlers::auth_router;
//!
//! let app = Router::new()
//!     .nest("/api/auth", auth_router())
//!     .with_state(your_app_state);
//! ```

#[cfg(feature = "axum")]
use std::sync::Arc;

#[cfg(feature = "axum")]
use axum::{
    extract::{Query, State},
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

#[cfg(feature = "axum")]
use axum_extra::extract::CookieJar;

#[cfg(feature = "axum")]
use serde::Deserialize;

#[cfg(feature = "axum")]
use crate::{
    oidc::{OidcProvider, OidcSessionStore},
    session::{self, SessionConfig},
    store::{AuthStore, AuthUser},
    token::{self, TokenConfig},
};

/// Shared state that the auth handlers need.
/// Wrap this in Arc and pass as Axum state or extension.
#[cfg(feature = "axum")]
pub struct AuthState<S: AuthStore> {
    pub provider: Option<OidcProvider>,
    pub sessions: OidcSessionStore,
    pub token_config: TokenConfig,
    pub session_config: SessionConfig,
    pub store: S,
}

#[cfg(feature = "axum")]
#[derive(Deserialize)]
struct CallbackQuery {
    code: String,
    state: String,
}

#[cfg(feature = "axum")]
#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

/// Build the auth router. Mount at `/api/auth`.
///
/// Provides:
/// - `GET  /oidc-config` - OIDC provider config for frontend
/// - `GET  /login/start` - Start OIDC flow (returns auth URL)
/// - `GET  /callback` - OIDC callback (sets cookies, redirects to /)
/// - `POST /login` - Password login (sets cookies)
/// - `POST /refresh` - Refresh session (reads/sets cookies)
/// - `POST /logout` - Clear session cookies
/// - `GET  /me` - Current user info (reads cookies)
#[cfg(feature = "axum")]
pub fn auth_router<S: AuthStore + 'static>() -> Router<Arc<AuthState<S>>> {
    Router::new()
        .route("/oidc-config", get(oidc_config::<S>))
        .route("/login/start", get(login_start::<S>))
        .route("/callback", get(callback::<S>))
        .route("/login", post(password_login::<S>))
        .route("/refresh", post(refresh::<S>))
        .route("/logout", post(logout::<S>))
        .route("/me", get(me::<S>))
}

/// Return OIDC config the frontend needs to show a login button.
#[cfg(feature = "axum")]
async fn oidc_config<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
) -> impl IntoResponse {
    match &state.provider {
        Some(p) => Json(p.frontend_config()).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
            "error": "OIDC not configured"
        }))).into_response(),
    }
}

/// Start the OIDC flow. Returns a JSON with the authorization URL.
/// Frontend redirects the browser to this URL.
#[cfg(feature = "axum")]
async fn login_start<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
) -> impl IntoResponse {
    let provider = match &state.provider {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
            "error": "OIDC not configured"
        }))).into_response(),
    };

    let (session_id, auth_url) = match state.sessions.start(provider, None).await {
        Ok(v) => v,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
            "error": "Too many pending login sessions, try again later"
        }))).into_response(),
    };

    Json(serde_json::json!({
        "session_id": session_id,
        "auth_url": auth_url,
    })).into_response()
}

/// OIDC callback. Exchanges code for tokens, creates user, sets cookies, redirects to /.
#[cfg(feature = "axum")]
async fn callback<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
    Query(params): Query<CallbackQuery>,
) -> impl IntoResponse {
    let provider = match &state.provider {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "OIDC not configured").into_response(),
    };

    // Find the session by state parameter
    let session = match state.sessions.get_by_state(&params.state).await {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, "Invalid or expired OIDC state").into_response(),
    };

    // Exchange code for tokens + user info
    let (_token_resp, user_info) = match provider
        .exchange_code(&params.code, &session.code_verifier)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "OIDC code exchange failed");
            return (StatusCode::BAD_REQUEST, "Authentication failed").into_response();
        }
    };

    // Clean up session
    state.sessions.remove(&session.id).await;

    // Upsert user via the project's AuthStore implementation
    let user = match state.store.upsert_user(&user_info).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create/update user");
            return (StatusCode::INTERNAL_SERVER_ERROR, "User provisioning failed").into_response();
        }
    };

    // Issue tokens
    let (access_token, _) = match token::issue_access_token(
        &state.token_config, &user.id, &user.email, &user.name, &user.role, &user.permissions,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "Failed to issue access token");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed").into_response();
        }
    };

    let refresh_token = token::generate_refresh_token();
    let refresh_hash = token::hash_refresh_token(&refresh_token);
    let expires = token::refresh_token_expiry(&state.token_config);

    if let Err(e) = state.store.store_refresh_token(&user.id, &refresh_hash, expires).await {
        tracing::error!(error = %e, "Failed to store refresh token");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Session creation failed").into_response();
    }

    let _ = state.store.on_login(&user.id).await;

    // Build redirect response with cookies
    let mut response = Response::builder().status(StatusCode::FOUND);
    session::set_session_cookies(
        &mut response, &state.session_config, &state.token_config,
        &access_token, &refresh_token,
    );

    response
        .header("Location", "/")
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

/// Password-based login. Sets cookies on success.
#[cfg(feature = "axum")]
async fn password_login<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    let user = match state.store.verify_password(&body.email, &body.password).await {
        Ok(Some(u)) => u,
        Ok(None) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Invalid email or password"
        }))).into_response(),
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Invalid email or password"
        }))).into_response(),
    };

    issue_session_response(&state, &user).await
}

/// Refresh the session. Reads refresh token from cookie, issues new tokens.
#[cfg(feature = "axum")]
async fn refresh<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
    jar: CookieJar,
) -> impl IntoResponse {
    let refresh_token = match session::extract_refresh_token(&jar, &state.session_config) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "No refresh token"
        }))).into_response(),
    };

    let hash = token::hash_refresh_token(&refresh_token);

    // Consume old refresh token (single-use)
    let user_id = match state.store.consume_refresh_token(&hash).await {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Invalid or expired refresh token"
        }))).into_response(),
    };

    // Load current user state
    let user = match state.store.get_user_by_id(&user_id).await {
        Ok(u) => u,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "User not found"
        }))).into_response(),
    };

    issue_session_response(&state, &user).await
}

/// Clear session cookies.
#[cfg(feature = "axum")]
async fn logout<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
    jar: CookieJar,
) -> impl IntoResponse {
    // Revoke refresh tokens if we can identify the user
    if let Some(access) = session::extract_access_token(&jar, &state.session_config) {
        if let Ok(claims) = token::validate_access_token(&access, &state.token_config.secret) {
            let _ = state.store.revoke_all_refresh_tokens(&claims.sub).await;
        }
    }

    let mut response = Response::builder().status(StatusCode::NO_CONTENT);
    session::clear_session_cookies(&mut response, &state.session_config);

    response
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

/// Get current user from session cookie.
#[cfg(feature = "axum")]
async fn me<S: AuthStore>(
    State(state): State<Arc<AuthState<S>>>,
    jar: CookieJar,
) -> impl IntoResponse {
    let access_token = match session::extract_access_token(&jar, &state.session_config) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Not authenticated"
        }))).into_response(),
    };

    let claims = match token::validate_access_token(&access_token, &state.token_config.secret) {
        Ok(c) => c,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "Invalid or expired session"
        }))).into_response(),
    };

    let user = match state.store.get_user_by_id(&claims.sub).await {
        Ok(u) => u,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "error": "User not found"
        }))).into_response(),
    };

    Json(serde_json::json!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
        "role": user.role,
        "avatar_url": user.avatar_url,
        "permissions": user.permissions,
    })).into_response()
}

/// Helper: issue tokens, set cookies, return JSON response.
#[cfg(feature = "axum")]
async fn issue_session_response<S: AuthStore>(
    state: &AuthState<S>,
    user: &AuthUser,
) -> axum::response::Response {
    let (access_token, _) = match token::issue_access_token(
        &state.token_config, &user.id, &user.email, &user.name, &user.role, &user.permissions,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "Failed to issue token");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "error": "Token generation failed"
            }))).into_response();
        }
    };

    let refresh_token = token::generate_refresh_token();
    let refresh_hash = token::hash_refresh_token(&refresh_token);
    let expires = token::refresh_token_expiry(&state.token_config);

    if let Err(e) = state.store.store_refresh_token(&user.id, &refresh_hash, expires).await {
        tracing::error!(error = %e, "Failed to store refresh token");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": "Session creation failed"
        }))).into_response();
    }

    let _ = state.store.on_login(&user.id).await;

    let mut response = Response::builder().status(StatusCode::OK);
    let csrf = session::set_session_cookies(
        &mut response, &state.session_config, &state.token_config,
        &access_token, &refresh_token,
    );

    let body = serde_json::json!({
        "user": {
            "id": user.id,
            "name": user.name,
            "email": user.email,
            "role": user.role,
            "avatar_url": user.avatar_url,
            "permissions": user.permissions,
        },
        "csrf_token": csrf,
    });

    response
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
        .into_response()
}
