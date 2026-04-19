//! Cookie-based session management for OIDC auth.
//!
//! Tokens are stored in httpOnly Secure cookies, never exposed to JavaScript.
//! The frontend just includes `credentials: "include"` on requests.
//!
//! ## Cookie layout
//!
//! | Cookie | httpOnly | Secure | SameSite | Contents |
//! |--------|----------|--------|----------|----------|
//! | `__Host-access` | yes | yes | Lax | JWT access token |
//! | `__Host-refresh` | yes | yes | Strict | opaque refresh token |
//! | `__Host-csrf` | no | yes | Lax | CSRF token (readable by JS) |
//!
//! The `__Host-` prefix enforces Secure + path=/ + no Domain (browser-enforced).
//!
//! ## CSRF protection
//!
//! SameSite=Lax prevents CSRF on state-changing requests from cross-origin.
//! For extra safety, the CSRF cookie value must be sent back as the
//! `X-CSRF-Token` header on POST/PUT/PATCH/DELETE requests.

#[cfg(feature = "axum")]
use axum::http::header::SET_COOKIE;

#[cfg(feature = "axum")]
use crate::token::TokenConfig;

/// Cookie name bases.
const ACCESS_NAME: &str = "access";
const REFRESH_NAME: &str = "refresh";
const CSRF_NAME: &str = "csrf";

/// Session configuration.
pub struct SessionConfig {
    /// Whether to set Secure flag (should be true in production).
    pub secure: bool,
    /// Cookie path (default: "/").
    pub path: String,
    /// Use `__Host-` prefix (requires Secure=true, no Domain, Path=/).
    /// Automatically disabled when `secure` is false (dev mode).
    use_host_prefix: bool,
}

impl SessionConfig {
    /// Production config: Secure cookies with `__Host-` prefix.
    pub fn production() -> Self {
        Self {
            secure: true,
            path: "/".into(),
            use_host_prefix: true,
        }
    }

    /// Dev-mode config: no Secure flag, no `__Host-` prefix (works on HTTP localhost).
    pub fn dev() -> Self {
        Self {
            secure: false,
            path: "/".into(),
            use_host_prefix: false,
        }
    }

    /// Get the effective cookie name for the access token.
    pub fn access_cookie_name(&self) -> String {
        self.prefixed(ACCESS_NAME)
    }

    /// Get the effective cookie name for the refresh token.
    pub fn refresh_cookie_name(&self) -> String {
        self.prefixed(REFRESH_NAME)
    }

    /// Get the effective cookie name for the CSRF token.
    pub fn csrf_cookie_name(&self) -> String {
        self.prefixed(CSRF_NAME)
    }

    fn prefixed(&self, name: &str) -> String {
        if self.use_host_prefix {
            format!("__Host-{name}")
        } else {
            name.to_string()
        }
    }

    /// Build a Set-Cookie header value for the access token.
    pub fn access_cookie(&self, token: &str, max_age_secs: i64) -> String {
        self.build_cookie(&self.access_cookie_name(), token, max_age_secs, true, "Lax")
    }

    /// Build a Set-Cookie header value for the refresh token.
    pub fn refresh_cookie(&self, token: &str, max_age_secs: i64) -> String {
        self.build_cookie(
            &self.refresh_cookie_name(),
            token,
            max_age_secs,
            true,
            "Strict",
        )
    }

    /// Build a Set-Cookie header value for the CSRF token.
    /// NOT httpOnly so JavaScript can read it.
    pub fn csrf_cookie(&self, token: &str) -> String {
        self.build_cookie(&self.csrf_cookie_name(), token, 86400 * 365, false, "Lax")
    }

    /// Build a clear cookie header (for logout).
    pub fn clear_cookie(&self, name: &str) -> String {
        self.build_cookie(name, "", 0, true, "Lax")
    }

    fn build_cookie(
        &self,
        name: &str,
        value: &str,
        max_age: i64,
        http_only: bool,
        same_site: &str,
    ) -> String {
        let mut parts = vec![format!("{name}={value}")];
        parts.push(format!("Path={}", self.path));
        parts.push(format!("Max-Age={max_age}"));
        parts.push(format!("SameSite={same_site}"));

        if http_only {
            parts.push("HttpOnly".into());
        }
        if self.secure {
            parts.push("Secure".into());
        }
        // Never set Domain for __Host- cookies (browser rejects them)

        parts.join("; ")
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self::production()
    }
}

/// Generate an HMAC-signed CSRF token bound to the user's session.
///
/// Format: `{random}.{hmac}` where HMAC = HMAC-SHA256(secret, random + access_token_hash).
/// This prevents subdomain cookie injection attacks because an attacker
/// cannot forge a valid HMAC without the server secret.
pub fn generate_csrf_token(secret: &str, access_token: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::{Digest, Sha256};

    type HmacSha256 = Hmac<Sha256>;

    // Random component
    let random = hex::encode(rand::random::<[u8; 16]>());

    // Hash the access token (don't include raw token in CSRF value)
    let mut token_hasher = Sha256::new();
    token_hasher.update(access_token.as_bytes());
    let token_hash = hex::encode(token_hasher.finalize());

    // HMAC-SHA256(secret, random + token_hash)
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(random.as_bytes());
    mac.update(token_hash.as_bytes());
    let hmac_result = hex::encode(mac.finalize().into_bytes());

    format!("{random}.{hmac_result}")
}

/// Verify an HMAC-signed CSRF token.
pub fn verify_csrf_token(csrf_token: &str, secret: &str, access_token: &str) -> bool {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::{Digest, Sha256};

    type HmacSha256 = Hmac<Sha256>;

    let parts: Vec<&str> = csrf_token.splitn(2, '.').collect();
    if parts.len() != 2 {
        return false;
    }
    let (random, provided_hmac) = (parts[0], parts[1]);

    // Recompute the expected HMAC
    let mut token_hasher = Sha256::new();
    token_hasher.update(access_token.as_bytes());
    let token_hash = hex::encode(token_hasher.finalize());

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(random.as_bytes());
    mac.update(token_hash.as_bytes());
    let expected_hmac = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison
    if expected_hmac.len() != provided_hmac.len() {
        return false;
    }
    expected_hmac
        .as_bytes()
        .iter()
        .zip(provided_hmac.as_bytes().iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Set session cookies on an Axum response after successful login/callback.
///
/// Sets: access token cookie, refresh token cookie, CSRF cookie.
/// Returns the CSRF token value (for the frontend to read from the cookie).
#[cfg(feature = "axum")]
pub fn set_session_cookies(
    response: &mut axum::http::response::Builder,
    session_config: &SessionConfig,
    token_config: &TokenConfig,
    access_token: &str,
    refresh_token: &str,
) -> String {
    let csrf = generate_csrf_token(&token_config.secret, access_token);

    let headers = response.headers_mut().unwrap();
    headers.append(
        SET_COOKIE,
        session_config
            .access_cookie(access_token, token_config.access_token_ttl)
            .parse()
            .unwrap(),
    );
    headers.append(
        SET_COOKIE,
        session_config
            .refresh_cookie(refresh_token, token_config.refresh_token_ttl)
            .parse()
            .unwrap(),
    );
    headers.append(
        SET_COOKIE,
        session_config.csrf_cookie(&csrf).parse().unwrap(),
    );

    csrf
}

/// Clear all session cookies (for logout).
#[cfg(feature = "axum")]
pub fn clear_session_cookies(
    response: &mut axum::http::response::Builder,
    session_config: &SessionConfig,
) {
    let headers = response.headers_mut().unwrap();
    for name in &[
        session_config.access_cookie_name(),
        session_config.refresh_cookie_name(),
        session_config.csrf_cookie_name(),
    ] {
        headers.append(
            SET_COOKIE,
            session_config.clear_cookie(name).parse().unwrap(),
        );
    }
}

/// Extract the access token from cookies in an Axum request.
#[cfg(feature = "axum")]
pub fn extract_access_token(
    cookies: &axum_extra::extract::CookieJar,
    config: &SessionConfig,
) -> Option<String> {
    cookies
        .get(&config.access_cookie_name())
        .map(|c| c.value().to_string())
}

/// Extract the refresh token from cookies in an Axum request.
#[cfg(feature = "axum")]
pub fn extract_refresh_token(
    cookies: &axum_extra::extract::CookieJar,
    config: &SessionConfig,
) -> Option<String> {
    cookies
        .get(&config.refresh_cookie_name())
        .map(|c| c.value().to_string())
}

/// Validate CSRF token: compare the cookie value with the X-CSRF-Token header.
/// Only required for state-changing methods (POST, PUT, PATCH, DELETE).
#[cfg(feature = "axum")]
pub fn validate_csrf(
    cookies: &axum_extra::extract::CookieJar,
    headers: &axum::http::HeaderMap,
    config: &SessionConfig,
) -> bool {
    let cookie_csrf = cookies
        .get(&config.csrf_cookie_name())
        .map(|c| c.value().to_string());
    let header_csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    match (cookie_csrf, header_csrf) {
        (Some(c), Some(h)) if !c.is_empty() && c.len() == h.len() => {
            // Constant-time comparison (length checked first to avoid prefix matching via zip)
            c.as_bytes()
                .iter()
                .zip(h.as_bytes().iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        _ => false,
    }
}
