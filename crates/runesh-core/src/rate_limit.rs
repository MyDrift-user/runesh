//! Sliding window rate limiter with in-memory and Redis backends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// In-memory sliding window rate limiter per key (typically IP address).
#[derive(Clone)]
pub struct InMemoryRateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl InMemoryRateLimiter {
    /// Create a new in-memory rate limiter.
    ///
    /// - `max_requests`: maximum requests allowed per key within the window.
    /// - `window_secs`: window duration in seconds.
    ///
    /// SECURITY: This constructor also spawns a background task that calls
    /// [`Self::cleanup`] every `window_secs` seconds to evict expired keys.
    /// Without it, the `requests` map would grow unbounded under attack
    /// (one entry per unique source IP, never freed). The task terminates
    /// automatically when the last `InMemoryRateLimiter` clone is dropped.
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        let this = Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
        };
        // Spawn cleanup loop tied to the Arc's lifetime.
        let weak = Arc::downgrade(&this.requests);
        let window = this.window;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(window);
            interval.tick().await; // first tick fires immediately — skip
            loop {
                interval.tick().await;
                let Some(map) = weak.upgrade() else { break };
                let now = Instant::now();
                let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
                guard.retain(|_, timestamps| {
                    timestamps.retain(|t| now.duration_since(*t) < window);
                    !timestamps.is_empty()
                });
            }
        });
        this
    }

    /// Check whether the given key is within the rate limit.
    /// Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.requests.lock().unwrap_or_else(|e| e.into_inner());

        // Prune expired timestamps for this key
        if let Some(timestamps) = map.get_mut(key) {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            if timestamps.is_empty() {
                map.remove(key);
            }
        }

        let timestamps = map.entry(key.to_string()).or_default();

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

/// Redis-backed distributed sliding window rate limiter.
///
/// Uses a Lua script with sorted sets for atomic sliding window counting.
/// Suitable for multi-pod deployments where all instances share the same
/// rate limit state via Redis.
#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisRateLimiter {
    pool: deadpool_redis::Pool,
    max_requests: usize,
    window_ms: u64,
    key_prefix: String,
}

#[cfg(feature = "redis")]
impl RedisRateLimiter {
    /// Create a new Redis-backed rate limiter.
    ///
    /// - `pool`: a deadpool-redis connection pool.
    /// - `max_requests`: maximum requests allowed per key within the window.
    /// - `window_secs`: window duration in seconds.
    /// - `key_prefix`: Redis key prefix (e.g. `"rl:"`) to namespace rate limit keys.
    pub fn new(
        pool: deadpool_redis::Pool,
        max_requests: usize,
        window_secs: u64,
        key_prefix: &str,
    ) -> Self {
        Self {
            pool,
            max_requests,
            window_ms: window_secs * 1000,
            key_prefix: key_prefix.to_string(),
        }
    }

    /// Check whether the given key is within the rate limit.
    /// Returns `true` if allowed, `false` if rate limited.
    ///
    /// Uses a Lua script for atomic sliding window with sorted sets:
    /// 1. ZREMRANGEBYSCORE to prune entries outside the window
    /// 2. ZCARD to count current entries
    /// 3. ZADD + EXPIRE if under limit
    pub async fn check(&self, key: &str) -> bool {
        let lua_script = r#"
local key = KEYS[1]
local now = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local limit = tonumber(ARGV[3])

redis.call('ZREMRANGEBYSCORE', key, 0, now - window)
local count = redis.call('ZCARD', key)
if count < limit then
    local seq = redis.call('INCR', key .. ':seq')
    redis.call('ZADD', key, now, now .. '-' .. seq)
    redis.call('EXPIRE', key, math.ceil(window / 1000))
    redis.call('EXPIRE', key .. ':seq', math.ceil(window / 1000))
    return 1
end
return 0
"#;

        let redis_key = format!("{}{}", self.key_prefix, key);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "Failed to get Redis connection for rate limiting, allowing request");
                return true; // Fail open: allow request if Redis is down
            }
        };

        let result: Result<i64, _> = deadpool_redis::redis::cmd("EVAL")
            .arg(lua_script)
            .arg(1) // number of keys
            .arg(&redis_key)
            .arg(now_ms)
            .arg(self.window_ms)
            .arg(self.max_requests as u64)
            .query_async(&mut *conn)
            .await;

        match result {
            Ok(1) => true,
            Ok(_) => false,
            Err(e) => {
                tracing::error!(error = %e, "Redis rate limit script failed, allowing request");
                true // Fail open
            }
        }
    }
}

/// Unified rate limiter backend supporting both in-memory and Redis.
#[derive(Clone)]
pub enum RateLimiterBackend {
    /// Single-instance in-memory rate limiter.
    InMemory(InMemoryRateLimiter),
    /// Distributed Redis-backed rate limiter.
    #[cfg(feature = "redis")]
    Redis(RedisRateLimiter),
}

impl RateLimiterBackend {
    /// Check whether the given key is within the rate limit.
    pub async fn check(&self, key: &str) -> bool {
        match self {
            Self::InMemory(limiter) => limiter.check(key),
            #[cfg(feature = "redis")]
            Self::Redis(limiter) => limiter.check(key).await,
        }
    }
}

/// Backwards-compatible type alias.
pub type RateLimiter = InMemoryRateLimiter;

/// Extract the client IP from request headers.
///
/// # Security
///
/// This function is only as trustworthy as your deployment topology. There are
/// two modes:
///
/// - `trust_proxy = false` (default): returns the direct socket peer (from
///   `axum::extract::ConnectInfo`). Cannot be spoofed because the client is
///   directly connected. Use this when the server is exposed to the public
///   internet without any proxy in front.
///
/// - `trust_proxy = true`: returns the **leftmost** entry of `X-Forwarded-For`,
///   which per [RFC 7239][rfc] is the original client. **This is only correct
///   if your reverse proxy strips any inbound `X-Forwarded-For` header before
///   appending its own**, otherwise an attacker can prepend a fake IP and spoof
///   rate-limit keys, audit logs, and any IP-based ACLs. Caddy's `reverse_proxy`
///   and nginx with `real_ip_header X-Forwarded-For; real_ip_recursive on;`
///   both do this correctly by default; bare `proxy_pass` in nginx does **not**.
///
/// [rfc]: https://datatracker.ietf.org/doc/html/rfc7239
#[cfg(feature = "axum")]
pub fn extract_client_ip(req: &axum::extract::Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(forwarded) = req.headers().get("x-forwarded-for") {
            if let Ok(val) = forwarded.to_str() {
                // Leftmost entry is the original client per RFC 7239.
                // Relies on the trusted proxy having stripped/rewritten any
                // inbound XFF header — see function docs.
                if let Some(ip) = val.split(',').next() {
                    let trimmed = ip.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }

        if let Some(real_ip) = req.headers().get("x-real-ip") {
            if let Ok(val) = real_ip.to_str() {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }

    // Fall through: direct socket address (cannot be spoofed).
    if let Some(connect_info) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        return connect_info.0.ip().to_string();
    }

    "unknown".to_string()
}

/// Axum rate limiting middleware factory using the unified backend.
///
/// Usage:
/// ```ignore
/// use axum::middleware;
/// use runesh_core::rate_limit::{rate_limit_layer, RateLimiterBackend, InMemoryRateLimiter};
///
/// let backend = RateLimiterBackend::InMemory(InMemoryRateLimiter::new(100, 60));
/// let app = Router::new()
///     .route("/api/v1/things", get(handler))
///     .layer(middleware::from_fn(move |req, next| {
///         let backend = backend.clone();
///         rate_limit_layer(backend, true, req, next)
///     }));
/// ```
#[cfg(feature = "axum")]
pub async fn rate_limit_layer(
    limiter: RateLimiterBackend,
    trust_proxy: bool,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let ip = extract_client_ip(&request, trust_proxy);

    if !limiter.check(&ip).await {
        tracing::warn!(ip = %ip, "Rate limit exceeded");
        return Err(axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}
