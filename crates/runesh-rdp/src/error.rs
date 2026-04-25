//! Error type returned across the public RDP API.

use thiserror::Error;

/// All failure modes the [`crate::RdpSession`] surface can emit.
///
/// `NotEnabled` is the precondition signal: it's the difference
/// between "we tried to RDP and the network rejected us" and "the
/// target machine has Remote Desktop turned off". Callers can offer
/// to flip the registry key for the operator instead of treating
/// the failure as opaque.
#[derive(Debug, Error)]
pub enum RdpError {
    /// `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\fDenyTSConnections`
    /// is `1` (Remote Desktop is disabled) on the target. The session
    /// cannot proceed; offer to enable RDP, then retry.
    #[error("Remote Desktop is disabled on the target (fDenyTSConnections=1)")]
    NotEnabled,

    /// Could not resolve `host:port` to a reachable socket.
    #[error("could not reach RDP target {host}:{port}: {source}")]
    Connect {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },

    /// TLS handshake failed. Most commonly: the server presented a
    /// certificate the client refused (only happens when
    /// `ignore_cert == false`).
    #[error("TLS handshake failed: {0}")]
    Tls(String),

    /// CredSSP / NTLM / Kerberos negotiation failed. Wraps the
    /// underlying `sspi` failure so callers can surface the
    /// human-readable reason ("wrong password", "account locked",
    /// "domain trust broken", etc.).
    #[error("CredSSP authentication failed: {0}")]
    Credssp(String),

    /// IronRDP returned an error during the connection handshake.
    #[error("RDP handshake error: {0}")]
    Handshake(String),

    /// An error during the active session loop after the connection
    /// was established. Usually means the server forcibly
    /// disconnected (admin kicked us, idle timeout, or a server-side
    /// crash).
    #[error("RDP session error: {0}")]
    Session(String),

    /// Encoder produced an error wrapping the H.264 sample.
    #[error("encoder: {0}")]
    Encoder(String),

    /// Generic IO error during read / write on the framed connection.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The active session task ended; no more frames will arrive.
    #[error("RDP session closed")]
    Closed,

    /// v1 placeholder. Returned everywhere the IronRDP wiring isn't
    /// landed yet so callers can pattern-match on a typed error
    /// rather than parsing string messages. Removed once the full
    /// session loop ships.
    #[error("RDP support is not implemented yet in this runesh release")]
    NotImplemented,
}
