//! Relay error types.

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid frame: {0}")]
    InvalidFrame(String),

    #[error("client disconnected")]
    Disconnected,

    #[error("unknown peer: {0}")]
    UnknownPeer(String),

    #[error("frame too large: {0} bytes (max {1})")]
    FrameTooLarge(usize, usize),

    #[error("authentication failed")]
    AuthFailed,

    #[error("server full")]
    ServerFull,

    #[error("handshake timeout")]
    HandshakeTimeout,

    #[error("protocol violation: {0}")]
    Protocol(String),
}
