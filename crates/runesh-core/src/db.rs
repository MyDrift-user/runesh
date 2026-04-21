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
    /// Server-side `statement_timeout` in milliseconds. Kills any single
    /// statement that runs longer than this. Default: 30_000 (30 s).
    pub statement_timeout_ms: u64,
    /// Server-side `lock_timeout` in milliseconds. Fails a statement if it
    /// can't acquire a lock within the window. Default: 10_000 (10 s).
    pub lock_timeout_ms: u64,
    /// Server-side `idle_in_transaction_session_timeout` in milliseconds.
    /// Closes connections that hold a transaction open but are idle.
    /// Default: 60_000 (60 s).
    pub idle_in_transaction_timeout_ms: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_secs: 5,
            idle_timeout_secs: 300,
            max_lifetime_secs: 1800,
            statement_timeout_ms: 30_000,
            lock_timeout_ms: 10_000,
            idle_in_transaction_timeout_ms: 60_000,
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
///
/// Installs per-connection timeouts in `after_connect` so runaway queries and
/// abandoned transactions can't monopolise a pool slot. Every new connection
/// runs `SET statement_timeout`, `SET lock_timeout`, and
/// `SET idle_in_transaction_session_timeout` before returning.
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

    let statement_timeout = config.statement_timeout_ms;
    let lock_timeout = config.lock_timeout_ms;
    let idle_tx_timeout = config.idle_in_transaction_timeout_ms;

    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
        .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                use sqlx::Executor;
                let stmt = format!(
                    "SET statement_timeout = {statement_timeout}; \
                     SET lock_timeout = {lock_timeout}; \
                     SET idle_in_transaction_session_timeout = {idle_tx_timeout};"
                );
                conn.execute(stmt.as_str()).await.map(|_| ())
            })
        })
        .connect(&database_url)
        .await
}
