//! Database pool setup helpers for SQLx + PostgreSQL.

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Create a PostgreSQL connection pool with production-ready defaults.
///
/// Reads `DATABASE_URL` from the environment if `url` is not provided.
/// Returns an error (never panics) if the URL is missing or connection fails.
pub async fn create_pool(url: Option<&str>) -> Result<PgPool, sqlx::Error> {
    let database_url = match url {
        Some(u) => u.to_string(),
        None => std::env::var("DATABASE_URL").map_err(|_| {
            sqlx::Error::Configuration("DATABASE_URL environment variable is not set".into())
        })?,
    };

    PgPoolOptions::new()
        .max_connections(20)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(&database_url)
        .await
}
