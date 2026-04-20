#![deny(unsafe_code)]
//! NAT traversal: STUN client, UDP hole punching, relay detection.

pub mod client;

pub use client::stun_binding_request;

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

/// NAT type as determined by STUN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NatType {
    /// No NAT (public IP).
    None,
    /// Full cone: any external host can reach the mapped address.
    FullCone,
    /// Restricted cone: only hosts the internal host has sent to can reach it.
    RestrictedCone,
    /// Port restricted: same as restricted, plus port must match.
    PortRestricted,
    /// Symmetric: each destination gets a different mapping.
    Symmetric,
    /// Could not determine.
    Unknown,
}

/// Result of a STUN binding request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StunResult {
    /// Public IP as seen by the STUN server.
    pub mapped_address: SocketAddr,
    /// Local address used.
    pub local_address: SocketAddr,
    /// Detected NAT type.
    pub nat_type: NatType,
    /// Round-trip time in milliseconds.
    pub rtt_ms: u64,
}

/// Endpoint candidate for peer connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub address: SocketAddr,
    pub candidate_type: CandidateType,
    pub priority: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateType {
    /// Direct local address.
    Host,
    /// STUN-discovered reflexive address.
    ServerReflexive,
    /// Relay address (TURN/DERP).
    Relay,
}

/// Connection strategy based on NAT types.
pub fn connection_strategy(local_nat: NatType, remote_nat: NatType) -> ConnectionMethod {
    match (local_nat, remote_nat) {
        (NatType::None, _) | (_, NatType::None) => ConnectionMethod::Direct,
        (NatType::FullCone, _) | (_, NatType::FullCone) => ConnectionMethod::Direct,
        (NatType::Symmetric, NatType::Symmetric) => ConnectionMethod::Relay,
        (NatType::Symmetric, _) | (_, NatType::Symmetric) => ConnectionMethod::HolePunchWithRelay,
        _ => ConnectionMethod::HolePunch,
    }
}

/// How to establish the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMethod {
    /// Direct UDP connection possible.
    Direct,
    /// UDP hole punching likely to succeed.
    HolePunch,
    /// Hole punching with relay fallback.
    HolePunchWithRelay,
    /// Must use relay (symmetric NAT on both sides).
    Relay,
}

#[derive(Debug, thiserror::Error)]
pub enum StunError {
    #[error("STUN request timeout")]
    Timeout,
    #[error("STUN server error: {0}")]
    ServerError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_when_no_nat() {
        assert_eq!(
            connection_strategy(NatType::None, NatType::Symmetric),
            ConnectionMethod::Direct
        );
        assert_eq!(
            connection_strategy(NatType::FullCone, NatType::RestrictedCone),
            ConnectionMethod::Direct
        );
    }

    #[test]
    fn relay_for_double_symmetric() {
        assert_eq!(
            connection_strategy(NatType::Symmetric, NatType::Symmetric),
            ConnectionMethod::Relay
        );
    }

    #[test]
    fn hole_punch_for_restricted() {
        assert_eq!(
            connection_strategy(NatType::RestrictedCone, NatType::PortRestricted),
            ConnectionMethod::HolePunch
        );
    }

    #[test]
    fn hole_punch_with_relay_for_mixed() {
        assert_eq!(
            connection_strategy(NatType::Symmetric, NatType::RestrictedCone),
            ConnectionMethod::HolePunchWithRelay
        );
    }

    #[test]
    fn stun_result_serialization() {
        let r = StunResult {
            mapped_address: "1.2.3.4:51820".parse().unwrap(),
            local_address: "192.168.1.5:51820".parse().unwrap(),
            nat_type: NatType::RestrictedCone,
            rtt_ms: 15,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: StunResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nat_type, NatType::RestrictedCone);
    }

    #[test]
    fn candidate_types() {
        for ct in [
            CandidateType::Host,
            CandidateType::ServerReflexive,
            CandidateType::Relay,
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let parsed: CandidateType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, ct);
        }
    }
}
