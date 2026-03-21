//! In-memory sliding window rate limiter.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sliding window rate limiter per key (typically IP address).
#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `max_requests`: maximum requests allowed per key within the window.
    /// - `window_secs`: window duration in seconds.
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    /// Check whether the given key is within the rate limit.
    /// Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.requests.lock().unwrap_or_else(|e| e.into_inner());

        let timestamps = map.entry(key.to_string()).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests {
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Remove all expired entries to free memory. Call periodically.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut map = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            !timestamps.is_empty()
        });
    }
}

/// Extract the client IP from request headers.
/// Checks X-Forwarded-For, X-Real-IP, then falls back to connect info.
#[cfg(feature = "axum")]
pub fn extract_client_ip(req: &axum::extract::Request) -> String {
    if let Some(forwarded) = req.headers().get("x-forwarded-for") {
        if let Ok(val) = forwarded.to_str() {
            if let Some(ip) = val.split(',').next() {
                return ip.trim().to_string();
            }
        }
    }

    if let Some(real_ip) = req.headers().get("x-real-ip") {
        if let Ok(val) = real_ip.to_str() {
            return val.trim().to_string();
        }
    }

    if let Some(connect_info) =
        req.extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        return connect_info.0.ip().to_string();
    }

    "unknown".to_string()
}

/// Axum rate limiting middleware factory.
///
/// Usage:
/// ```ignore
/// use axum::middleware;
/// use runesh_core::rate_limit::{rate_limit_layer, RateLimiter};
///
/// let limiter = RateLimiter::new(100, 60); // 100 req/min
/// let app = Router::new()
///     .route("/api/v1/things", get(handler))
///     .layer(middleware::from_fn(move |req, next| {
///         let limiter = limiter.clone();
///         rate_limit_layer(limiter, req, next)
///     }));
/// ```
#[cfg(feature = "axum")]
pub async fn rate_limit_layer(
    limiter: RateLimiter,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let ip = extract_client_ip(&request);

    if !limiter.check(&ip) {
        tracing::warn!(ip = %ip, "Rate limit exceeded");
        return Err(axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}
