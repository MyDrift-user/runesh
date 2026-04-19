//! Proxy error types.

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("no route for host: {0}")]
    NoRoute(String),

    #[error("backend unreachable: {0}")]
    BackendUnreachable(String),

    #[error("access denied: {0}")]
    AccessDenied(String),

    #[error("certificate error: {0}")]
    Certificate(String),

    #[error("invalid resource config: {0}")]
    InvalidConfig(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
