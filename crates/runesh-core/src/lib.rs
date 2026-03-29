pub mod error;
pub mod pagination;
pub mod rate_limit;
pub mod shutdown;
pub mod ws_broadcast;
pub mod upload;
pub mod service;

#[cfg(feature = "axum")]
pub mod middleware;

#[cfg(feature = "redis")]
pub mod redis;

#[cfg(feature = "sqlx")]
pub mod db;

#[cfg(feature = "metrics")]
pub mod metrics;

#[cfg(feature = "openapi")]
pub mod openapi;

pub use error::AppError;
pub use pagination::{Pagination, PaginatedResponse};
pub use rate_limit::{RateLimiter, InMemoryRateLimiter, RateLimiterBackend};
#[cfg(feature = "redis")]
pub use rate_limit::RedisRateLimiter;
#[cfg(feature = "sqlx")]
pub use db::PoolConfig;
pub use shutdown::{shutdown_signal, graceful_shutdown, ShutdownRegistry};
pub use upload::validate_magic_bytes;
pub use ws_broadcast::WsLimits;
#[cfg(feature = "redis")]
pub use ws_broadcast::RedisBroadcastRegistry;
