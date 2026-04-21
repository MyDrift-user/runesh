//! Coordination server error types.

#[derive(Debug, thiserror::Error)]
pub enum CoordError {
    #[error("noise handshake failed: {0}")]
    Handshake(String),

    #[error("node not found: {0}")]
    NodeNotFound(String),

    #[error("node not authorized: {0}")]
    NotAuthorized(String),

    #[error("invalid machine key")]
    InvalidMachineKey,

    #[error("invalid node key")]
    InvalidNodeKey,

    #[error("registration failed: {0}")]
    Registration(String),

    #[error("tag not owned by identity: {0}")]
    UnauthorizedTag(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("snow error: {0}")]
    Snow(#[from] snow::Error),
}
