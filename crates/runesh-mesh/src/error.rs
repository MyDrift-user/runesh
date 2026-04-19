//! Mesh error types.

#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("IP pool exhausted for tenant {0}")]
    IpPoolExhausted(String),

    #[error("duplicate peer: {0}")]
    DuplicatePeer(String),

    #[error("tunnel error: {0}")]
    Tunnel(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
}
