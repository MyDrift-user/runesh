//! ACL error types.

#[derive(Debug, thiserror::Error)]
pub enum AclError {
    #[error("invalid HuJSON: {0}")]
    InvalidHuJson(String),

    #[error("invalid ACL policy: {0}")]
    InvalidPolicy(String),

    #[error("unknown group: {0}")]
    UnknownGroup(String),

    #[error("unknown host alias: {0}")]
    UnknownHost(String),

    #[error("invalid port range: {0}")]
    InvalidPortRange(String),

    #[error("invalid CIDR: {0}")]
    InvalidCidr(String),

    #[error("circular group reference: {0}")]
    CircularGroup(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
