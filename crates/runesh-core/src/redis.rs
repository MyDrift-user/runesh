//! Redis connection pool setup using deadpool-redis.

pub use deadpool_redis::{Config, Connection, Pool, Runtime};

/// Create a Redis connection pool from a URL.
///
/// Reads `REDIS_URL` from the environment if `url` is not provided.
/// Returns an error (never panics) if the URL is missing or pool creation fails.
///
/// # Example
/// ```ignore
/// let pool = create_redis_pool(Some("redis://127.0.0.1:6379")).unwrap();
/// let mut conn = pool.get().await.unwrap();
/// ```
pub fn create_redis_pool(url: Option<&str>) -> Result<Pool, deadpool_redis::CreatePoolError> {
    let redis_url = match url {
        Some(u) => u.to_string(),
        None => std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
    };

    let cfg = Config::from_url(&redis_url);
    cfg.create_pool(Some(Runtime::Tokio1))
}
