pub mod error;
pub mod pagination;
pub mod rate_limit;
pub mod shutdown;
pub mod ws_broadcast;
pub mod upload;
pub mod service;

#[cfg(feature = "axum")]
pub mod middleware;

#[cfg(feature = "sqlx")]
pub mod db;

pub use error::AppError;
pub use pagination::{Pagination, PaginatedResponse};
pub use rate_limit::RateLimiter;
pub use shutdown::shutdown_signal;
pub use upload::validate_magic_bytes;
pub use ws_broadcast::WsLimits;
