//! Shared Axum middleware: request ID, structured logging, CORS, security headers, health check.

#[cfg(feature = "axum")]
pub mod request_id {
    use axum::{
        body::Body, extract::Request, http::HeaderValue, middleware::Next, response::Response,
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
    use axum::{body::Body, extract::Request, middleware::Next, response::Response};
    use std::time::Instant;

    /// Middleware that logs each request with method, path, status, and latency.
    ///
    /// When the `metrics` feature is enabled, also records:
    /// - `http_requests_total` counter (labels: method, path, status)
    /// - `http_request_duration_seconds` histogram (labels: method, path)
    pub async fn logging_middleware(req: Request<Body>, next: Next) -> Response {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let start = Instant::now();

        let response = next.run(req).await;

        let duration = start.elapsed();
        let latency = duration.as_millis();
        let status = response.status().as_u16();

        // Record Prometheus metrics when the feature is enabled
        #[cfg(feature = "metrics")]
        {
            let labels = [
                ("method", method.clone()),
                ("path", path.clone()),
                ("status", status.to_string()),
            ];
            metrics::counter!("http_requests_total", &labels).increment(1);

            let hist_labels = [("method", method.clone()), ("path", path.clone())];
            metrics::histogram!("http_request_duration_seconds", &hist_labels)
                .record(duration.as_secs_f64());
        }

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
            tracing::warn!(
                "CORS configured with wildcard origin -- do not use in production with credentials"
            );
            layer
                .allow_origin(tower_http::cors::Any)
                .allow_credentials(false)
        } else {
            let parsed: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
            layer.allow_origin(parsed).allow_credentials(true)
        }
    }
}

#[cfg(feature = "axum")]
pub mod security_headers {
    use axum::{body::Body, extract::Request, middleware::Next, response::Response};
    use std::sync::Arc;

    /// Builder for the Content-Security-Policy header.
    ///
    /// The secure default denies `'unsafe-inline'` and `'unsafe-eval'` on every
    /// directive. SPAs that legitimately require inline styles (Tailwind/Next.js)
    /// can opt in to `'unsafe-inline'` on `style-src` only via
    /// [`CspConfig::allow_inline_styles`]. Inline scripts are NEVER enabled by
    /// default; callers that truly need them must call
    /// [`CspConfig::allow_inline_scripts`] and accept the XSS risk.
    #[derive(Debug, Clone)]
    pub struct CspConfig {
        /// If true, `style-src` includes `'unsafe-inline'`. Default: false.
        pub allow_inline_styles: bool,
        /// If true, `script-src` includes `'unsafe-inline'`. Default: false.
        /// Do NOT enable without a compelling reason (breaks XSS defense).
        pub allow_inline_scripts: bool,
        /// If true, `script-src` includes `'unsafe-eval'`. Default: false.
        /// Do NOT enable without a compelling reason.
        pub allow_eval: bool,
        /// Extra host sources appended to `style-src`. e.g.
        /// `["https://fonts.googleapis.com"]`. Default: fonts.googleapis.com.
        pub style_src_hosts: Vec<String>,
        /// Extra host sources appended to `font-src`. Default: fonts.gstatic.com.
        pub font_src_hosts: Vec<String>,
        /// Extra host sources appended to `img-src`. Default: `data:`, `blob:`, `https:`.
        pub img_src_hosts: Vec<String>,
        /// Extra host sources appended to `connect-src`. Default: `ws:`, `wss:`.
        pub connect_src_hosts: Vec<String>,
        /// Extra host sources appended to `media-src`. Default: `blob:`.
        pub media_src_hosts: Vec<String>,
        /// Extra host sources appended to `frame-src`. Default: none.
        pub frame_src_hosts: Vec<String>,
    }

    impl Default for CspConfig {
        fn default() -> Self {
            Self {
                allow_inline_styles: false,
                allow_inline_scripts: false,
                allow_eval: false,
                style_src_hosts: vec!["https://fonts.googleapis.com".into()],
                font_src_hosts: vec!["https://fonts.gstatic.com".into()],
                img_src_hosts: vec!["data:".into(), "blob:".into(), "https:".into()],
                connect_src_hosts: vec!["ws:".into(), "wss:".into()],
                media_src_hosts: vec!["blob:".into()],
                frame_src_hosts: vec![],
            }
        }
    }

    impl CspConfig {
        /// Secure default: no inline scripts, no eval, no inline styles.
        pub fn strict() -> Self {
            Self {
                allow_inline_styles: false,
                allow_inline_scripts: false,
                allow_eval: false,
                ..Self::default()
            }
        }

        /// Relaxed for Next.js / Tailwind style systems: permits inline styles
        /// (hashing every style attribute is impractical) but keeps scripts strict.
        pub fn spa_styles() -> Self {
            Self {
                allow_inline_styles: true,
                allow_inline_scripts: false,
                allow_eval: false,
                ..Self::default()
            }
        }

        /// Render the CSP header value.
        pub fn to_header_value(&self) -> String {
            let style_extras = self.style_src_hosts.join(" ");
            let font_extras = self.font_src_hosts.join(" ");
            let img_extras = self.img_src_hosts.join(" ");
            let connect_extras = self.connect_src_hosts.join(" ");
            let media_extras = self.media_src_hosts.join(" ");
            let frame_extras = self.frame_src_hosts.join(" ");

            let script_inline = if self.allow_inline_scripts {
                " 'unsafe-inline'"
            } else {
                ""
            };
            let script_eval = if self.allow_eval {
                " 'unsafe-eval'"
            } else {
                ""
            };
            let style_inline = if self.allow_inline_styles {
                " 'unsafe-inline'"
            } else {
                ""
            };

            let mut out = String::new();
            out.push_str("default-src 'self'; ");
            out.push_str(&format!("script-src 'self'{script_inline}{script_eval}; "));
            if style_extras.is_empty() {
                out.push_str(&format!("style-src 'self'{style_inline}; "));
            } else {
                out.push_str(&format!("style-src 'self'{style_inline} {style_extras}; "));
            }
            if font_extras.is_empty() {
                out.push_str("font-src 'self'; ");
            } else {
                out.push_str(&format!("font-src 'self' {font_extras}; "));
            }
            if img_extras.is_empty() {
                out.push_str("img-src 'self'; ");
            } else {
                out.push_str(&format!("img-src 'self' {img_extras}; "));
            }
            if media_extras.is_empty() {
                out.push_str("media-src 'self'; ");
            } else {
                out.push_str(&format!("media-src 'self' {media_extras}; "));
            }
            if connect_extras.is_empty() {
                out.push_str("connect-src 'self'; ");
            } else {
                out.push_str(&format!("connect-src 'self' {connect_extras}; "));
            }
            if frame_extras.is_empty() {
                out.push_str("frame-src 'self'");
            } else {
                out.push_str(&format!("frame-src 'self' {frame_extras}"));
            }
            out
        }
    }

    /// Middleware that sets recommended security headers on every response.
    ///
    /// Uses the secure default CSP ([`CspConfig::strict`]) which blocks inline
    /// scripts, styles, and `eval`. To customize, use
    /// [`security_headers_middleware_with`] instead.
    ///
    /// Headers set:
    /// - `X-Content-Type-Options: nosniff` (prevents MIME sniffing)
    /// - `X-Frame-Options: DENY` (prevents clickjacking)
    /// - `Strict-Transport-Security: max-age=31536000; includeSubDomains` (HSTS)
    /// - `Referrer-Policy: strict-origin-when-cross-origin`
    /// - `Permissions-Policy: camera=(), microphone=(), geolocation=()`
    /// - `X-XSS-Protection: 0` (disable legacy XSS filter to prevent false positives)
    /// - `Content-Security-Policy` (restricts resource loading origins)
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
        apply_security_headers(req, next, &CspConfig::strict()).await
    }

    /// Like [`security_headers_middleware`] but uses a caller-supplied CSP.
    ///
    /// Build the layer from a `CspConfig` and plug it in with
    /// `axum::middleware::from_fn_with_state` or a closure.
    pub async fn security_headers_middleware_with(
        csp: Arc<CspConfig>,
        req: Request<Body>,
        next: Next,
    ) -> Response {
        apply_security_headers(req, next, &csp).await
    }

    async fn apply_security_headers(req: Request<Body>, next: Next, csp: &CspConfig) -> Response {
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

        let csp_value = csp.to_header_value();
        if let Ok(v) = csp_value.parse() {
            headers.insert("content-security-policy", v);
        }

        response
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn strict_csp_has_no_unsafe() {
            let csp = CspConfig::strict().to_header_value();
            assert!(!csp.contains("'unsafe-inline'"));
            assert!(!csp.contains("'unsafe-eval'"));
            assert!(csp.contains("script-src 'self'"));
        }

        #[test]
        fn spa_styles_allows_inline_style_only() {
            let csp = CspConfig::spa_styles().to_header_value();
            // script-src must remain strict
            let script_section = csp
                .split(';')
                .find(|s| s.trim().starts_with("script-src"))
                .unwrap();
            assert!(!script_section.contains("'unsafe-inline'"));
            assert!(!script_section.contains("'unsafe-eval'"));
            // style-src may include 'unsafe-inline'
            let style_section = csp
                .split(';')
                .find(|s| s.trim().starts_with("style-src"))
                .unwrap();
            assert!(style_section.contains("'unsafe-inline'"));
        }
    }
}

#[cfg(feature = "axum")]
pub mod health {
    use axum::{Json, extract::State, http::StatusCode};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Build version, set at compile time via CARGO_PKG_VERSION.
    const VERSION: &str = env!("CARGO_PKG_VERSION");

    /// Shared flag indicating the application has finished startup.
    /// Clone an `Arc<AtomicBool>` and set it to `true` once initialization is complete.
    pub type StartupFlag = Arc<AtomicBool>;

    /// Create a new startup flag (initially `false`).
    pub fn new_startup_flag() -> StartupFlag {
        Arc::new(AtomicBool::new(false))
    }

    /// Health check handler. Mount at `/health` or `/healthz`.
    ///
    /// Returns 200 when all checks pass, 503 when any check fails.
    /// Response includes version and per-check status.
    ///
    /// ```ignore
    /// let app = Router::new()
    ///     .route("/health", get(health_handler))
    ///     .with_state(pool);
    /// ```
    #[cfg(feature = "sqlx")]
    pub async fn health_handler(State(pool): State<sqlx::PgPool>) -> (StatusCode, Json<Value>) {
        let db_check = match sqlx::query("SELECT 1").execute(&pool).await {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        };

        let db_ok = db_check == "ok";
        let status = if db_ok { "ok" } else { "degraded" };
        let code = if db_ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        (
            code,
            Json(json!({
                "status": status,
                "version": VERSION,
                "checks": {
                    "database": db_check,
                    "redis": "not configured",
                },
            })),
        )
    }

    /// Health check handler with both database and Redis.
    ///
    /// Returns 200 when all checks pass, 503 when any check fails.
    #[cfg(all(feature = "sqlx", feature = "redis"))]
    pub async fn health_handler_with_redis(
        State((pool, redis_pool)): State<(sqlx::PgPool, deadpool_redis::Pool)>,
    ) -> (StatusCode, Json<Value>) {
        let db_check = match sqlx::query("SELECT 1").execute(&pool).await {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        };

        let redis_check = match redis_pool.get().await {
            Ok(mut conn) => {
                match deadpool_redis::redis::cmd("PING")
                    .query_async::<String>(&mut *conn)
                    .await
                {
                    Ok(_) => "ok".to_string(),
                    Err(e) => format!("error: {e}"),
                }
            }
            Err(e) => format!("error: {e}"),
        };

        let all_ok = db_check == "ok" && redis_check == "ok";
        let status = if all_ok { "ok" } else { "degraded" };
        let code = if all_ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        (
            code,
            Json(json!({
                "status": status,
                "version": VERSION,
                "checks": {
                    "database": db_check,
                    "redis": redis_check,
                },
            })),
        )
    }

    /// Readiness handler. Returns 200 only if ALL dependency checks pass.
    ///
    /// Mount at `/ready` or `/readyz`. Kubernetes uses this to decide whether
    /// to send traffic to the pod.
    #[cfg(feature = "sqlx")]
    pub async fn readiness_handler(State(pool): State<sqlx::PgPool>) -> (StatusCode, Json<Value>) {
        let db_check = match sqlx::query("SELECT 1").execute(&pool).await {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        };

        let all_ok = db_check == "ok";
        let code = if all_ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        (
            code,
            Json(json!({
                "status": if all_ok { "ready" } else { "not ready" },
                "version": VERSION,
                "checks": {
                    "database": db_check,
                },
            })),
        )
    }

    /// Simple liveness check (no DB dependency). Mount at `/livez`.
    ///
    /// Always returns 200. Kubernetes uses this to decide whether to restart
    /// the container — only fails if the process itself is unhealthy.
    pub async fn liveness_handler() -> Json<Value> {
        Json(json!({ "status": "ok" }))
    }

    /// Startup probe handler. Mount at `/startupz`.
    ///
    /// Returns 200 once the application has finished initialization.
    /// Returns 503 while still starting up. Kubernetes uses this to avoid
    /// killing slow-starting containers.
    pub async fn startup_handler(State(flag): State<StartupFlag>) -> (StatusCode, Json<Value>) {
        if flag.load(Ordering::Relaxed) {
            (
                StatusCode::OK,
                Json(json!({ "status": "started", "version": VERSION })),
            )
        } else {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "starting", "version": VERSION })),
            )
        }
    }
}
