pub mod error;
pub mod pagination;
pub mod rate_limit;
pub mod service;
pub mod shutdown;
pub mod upload;
pub mod ws_broadcast;

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

#[cfg(feature = "sqlx")]
pub use db::PoolConfig;
pub use error::AppError;
pub use pagination::{PaginatedResponse, Pagination};
#[cfg(feature = "redis")]
pub use rate_limit::RedisRateLimiter;
pub use rate_limit::{FailMode, InMemoryRateLimiter, RateLimiter, RateLimiterBackend};
pub use shutdown::{ShutdownRegistry, graceful_shutdown, shutdown_signal};
pub use upload::validate_magic_bytes;
#[cfg(feature = "redis")]
pub use ws_broadcast::RedisBroadcastRegistry;
pub use ws_broadcast::WsLimits;
