//! Database pool setup helpers for SQLx + PostgreSQL.

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Configuration for the PostgreSQL connection pool.
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of idle connections to maintain.
    pub min_connections: u32,
    /// Timeout in seconds when acquiring a connection from the pool.
    pub acquire_timeout_secs: u64,
    /// Duration in seconds before an idle connection is closed.
    pub idle_timeout_secs: u64,
    /// Maximum lifetime in seconds of a connection before it is recycled.
    pub max_lifetime_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_secs: 5,
            idle_timeout_secs: 300,
            max_lifetime_secs: 1800,
        }
    }
}

/// Create a PostgreSQL connection pool with production-ready defaults.
///
/// Reads `DATABASE_URL` from the environment if `url` is not provided.
/// Returns an error (never panics) if the URL is missing or connection fails.
pub async fn create_pool(url: Option<&str>) -> Result<PgPool, sqlx::Error> {
    create_pool_with_config(url, PoolConfig::default()).await
}

/// Create a PostgreSQL connection pool with custom configuration.
///
/// Reads `DATABASE_URL` from the environment if `url` is not provided.
/// Returns an error (never panics) if the URL is missing or connection fails.
pub async fn create_pool_with_config(
    url: Option<&str>,
    config: PoolConfig,
) -> Result<PgPool, sqlx::Error> {
    let database_url = match url {
        Some(u) => u.to_string(),
        None => std::env::var("DATABASE_URL").map_err(|_| {
            sqlx::Error::Configuration("DATABASE_URL environment variable is not set".into())
        })?,
    };

    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
        .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
        .connect(&database_url)
        .await
}
