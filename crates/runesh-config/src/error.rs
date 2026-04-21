//! Error types for [`ConfigApplier`](crate::ConfigApplier) impls.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    /// The spec is well-formed but this platform cannot enforce it.
    #[error("not supported on this platform: {0}")]
    NotSupported(String),

    /// The caller lacks permission to change the subsystem.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Spec validation failed (invalid hostname, empty username, etc.).
    #[error("invalid spec: {0}")]
    InvalidSpec(String),

    /// Underlying platform API returned an error.
    #[error("platform error: {0}")]
    Platform(String),

    /// I/O error reading or writing a config file.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
