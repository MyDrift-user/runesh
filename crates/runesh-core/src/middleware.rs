//! Shared Axum middleware: request ID, structured logging, CORS, security headers, health check.

#[cfg(feature = "axum")]
pub mod request_id {
    use axum::{
        body::Body,
        extract::Request,
        http::HeaderValue,
        middleware::Next,
        response::Response,
    };
    use uuid::Uuid;

    /// Middleware that ensures every request has a unique ID.
    ///
    /// Reads `X-Request-Id` from the incoming request or generates a UUID.
    /// Validates that incoming IDs are alphanumeric/dashes only (max 64 chars)
    /// to prevent log injection. Sets the ID on response headers and tracing span.
    pub async fn request_id_middleware(req: Request<Body>, next: Next) -> Response {
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| {
                s.len() <= 64
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let span = tracing::info_span!("request", id = %request_id);
        let _guard = span.enter();

        let mut response = next.run(req).await;
        if let Ok(val) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", val);
        }
        response
    }
}

#[cfg(feature = "axum")]
pub mod logging {
    use axum::{
        body::Body,
        extract::Request,
        middleware::Next,
        response::Response,
    };
    use std::time::Instant;

    /// Middleware that logs each request with method, path, status, and latency.
    pub async fn logging_middleware(req: Request<Body>, next: Next) -> Response {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let start = Instant::now();

        let response = next.run(req).await;

        let latency = start.elapsed().as_millis();
        let status = response.status().as_u16();

        if status >= 500 {
            tracing::error!(method = %method, path = %path, status, latency_ms = latency, "request");
        } else if status >= 400 {
            tracing::warn!(method = %method, path = %path, status, latency_ms = latency, "request");
        } else {
            tracing::info!(method = %method, path = %path, status, latency_ms = latency, "request");
        }

        response
    }
}

#[cfg(feature = "axum")]
pub mod cors {
    use axum::http::{HeaderValue, Method};
    use tower_http::cors::CorsLayer;

    /// Create a CORS layer with sensible defaults.
    ///
    /// - `origins`: allowed origins (use `["*"]` for dev, specific domains for prod)
    pub fn cors_layer(origins: &[&str]) -> CorsLayer {
        let layer = CorsLayer::new()
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                "Content-Type".parse().unwrap(),
                "Authorization".parse().unwrap(),
                "X-CSRF-Token".parse().unwrap(),
                "X-Request-Id".parse().unwrap(),
            ])
            .max_age(std::time::Duration::from_secs(3600));

        if origins.contains(&"*") {
            tracing::warn!("CORS configured with wildcard origin -- do not use in production with credentials");
            layer
                .allow_origin(tower_http::cors::Any)
                .allow_credentials(false)
        } else {
            let parsed: Vec<HeaderValue> = origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            layer
                .allow_origin(parsed)
                .allow_credentials(true)
        }
    }
}

#[cfg(feature = "axum")]
pub mod security_headers {
    use axum::{
        body::Body,
        extract::Request,
        middleware::Next,
        response::Response,
    };

    /// Middleware that sets recommended security headers on every response.
    ///
    /// Headers set:
    /// - `X-Content-Type-Options: nosniff` (prevents MIME sniffing)
    /// - `X-Frame-Options: DENY` (prevents clickjacking)
    /// - `Strict-Transport-Security: max-age=31536000; includeSubDomains` (HSTS)
    /// - `Referrer-Policy: strict-origin-when-cross-origin`
    /// - `Permissions-Policy: camera=(), microphone=(), geolocation=()`
    /// - `X-XSS-Protection: 0` (disable legacy XSS filter to prevent false positives)
    ///
    /// Usage:
    /// ```ignore
    /// use axum::middleware;
    /// use runesh_core::middleware::security_headers::security_headers_middleware;
    ///
    /// let app = Router::new()
    ///     .layer(middleware::from_fn(security_headers_middleware));
    /// ```
    pub async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
        let mut response = next.run(req).await;
        let headers = response.headers_mut();

        headers.insert("x-content-type-options", "nosniff".parse().unwrap());
        headers.insert("x-frame-options", "DENY".parse().unwrap());
        headers.insert(
            "strict-transport-security",
            "max-age=31536000; includeSubDomains".parse().unwrap(),
        );
        headers.insert(
            "referrer-policy",
            "strict-origin-when-cross-origin".parse().unwrap(),
        );
        headers.insert(
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()".parse().unwrap(),
        );
        headers.insert("x-xss-protection", "0".parse().unwrap());

        response
    }
}

#[cfg(feature = "axum")]
pub mod health {
    use axum::{extract::State, Json};
    use serde_json::{json, Value};

    /// Health check handler. Mount at `/health` or `/healthz`.
    ///
    /// ```ignore
    /// let app = Router::new()
    ///     .route("/health", get(health_handler))
    ///     .with_state(pool);
    /// ```
    #[cfg(feature = "sqlx")]
    pub async fn health_handler(
        State(pool): State<sqlx::PgPool>,
    ) -> Json<Value> {
        let db_ok = sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .is_ok();

        Json(json!({
            "status": if db_ok { "ok" } else { "degraded" },
            "db": if db_ok { "ok" } else { "error" },
        }))
    }

    /// Simple liveness check (no DB dependency).
    pub async fn liveness_handler() -> Json<Value> {
        Json(json!({ "status": "ok" }))
    }
}
