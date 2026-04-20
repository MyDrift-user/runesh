//! STUN binding request client (RFC 5389).
//!
//! Sends a minimal 20-byte STUN Binding Request over UDP and parses
//! the XOR-MAPPED-ADDRESS from the response to discover the public
//! IP and port as seen by the STUN server.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rand::RngCore;
use tokio::net::UdpSocket;

use crate::{NatType, StunError, StunResult};

/// STUN magic cookie (RFC 5389).
const MAGIC_COOKIE: u32 = 0x2112_A442;

/// STUN message type: Binding Request.
const BINDING_REQUEST: u16 = 0x0001;

/// STUN message type: Binding Success Response.
const BINDING_RESPONSE: u16 = 0x0101;

/// STUN attribute: XOR-MAPPED-ADDRESS.
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// STUN attribute: MAPPED-ADDRESS (fallback for old servers).
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;

/// Build a minimal STUN Binding Request (20 bytes).
fn build_binding_request() -> ([u8; 20], [u8; 12]) {
    let mut msg = [0u8; 20];

    // Message type: Binding Request (0x0001)
    msg[0] = (BINDING_REQUEST >> 8) as u8;
    msg[1] = (BINDING_REQUEST & 0xFF) as u8;

    // Message length: 0 (no attributes)
    msg[2] = 0;
    msg[3] = 0;

    // Magic cookie
    msg[4] = (MAGIC_COOKIE >> 24) as u8;
    msg[5] = (MAGIC_COOKIE >> 16) as u8;
    msg[6] = (MAGIC_COOKIE >> 8) as u8;
    msg[7] = (MAGIC_COOKIE & 0xFF) as u8;

    // Transaction ID: 12 random bytes
    let mut txn_id = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut txn_id);
    msg[8..20].copy_from_slice(&txn_id);

    (msg, txn_id)
}

/// Parse a STUN Binding Response, extracting the mapped address.
fn parse_binding_response(
    data: &[u8],
    expected_txn_id: &[u8; 12],
) -> Result<SocketAddr, StunError> {
    if data.len() < 20 {
        return Err(StunError::ServerError("response too short".into()));
    }

    // Check message type
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    if msg_type != BINDING_RESPONSE {
        return Err(StunError::ServerError(format!(
            "unexpected message type: 0x{msg_type:04x}"
        )));
    }

    // Check magic cookie
    let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if cookie != MAGIC_COOKIE {
        return Err(StunError::ServerError("bad magic cookie".into()));
    }

    // Check transaction ID
    if &data[8..20] != expected_txn_id {
        return Err(StunError::ServerError("transaction ID mismatch".into()));
    }

    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if data.len() < 20 + msg_len {
        return Err(StunError::ServerError("truncated response".into()));
    }

    // Parse attributes
    let mut offset = 20;
    while offset + 4 <= 20 + msg_len {
        let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;

        if offset + attr_len > data.len() {
            break;
        }

        if attr_type == ATTR_XOR_MAPPED_ADDRESS {
            return parse_xor_mapped_address(&data[offset..offset + attr_len]);
        }

        if attr_type == ATTR_MAPPED_ADDRESS {
            return parse_mapped_address(&data[offset..offset + attr_len]);
        }

        // Pad to 4-byte boundary
        offset += (attr_len + 3) & !3;
    }

    Err(StunError::ServerError(
        "no MAPPED-ADDRESS in response".into(),
    ))
}

/// Parse XOR-MAPPED-ADDRESS attribute.
fn parse_xor_mapped_address(data: &[u8]) -> Result<SocketAddr, StunError> {
    if data.len() < 8 {
        return Err(StunError::ServerError(
            "XOR-MAPPED-ADDRESS too short".into(),
        ));
    }

    let family = data[1];
    let xport = u16::from_be_bytes([data[2], data[3]]);
    let port = xport ^ (MAGIC_COOKIE >> 16) as u16;

    match family {
        0x01 => {
            // IPv4
            let xaddr = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let addr = xaddr ^ MAGIC_COOKIE;
            let ip = std::net::Ipv4Addr::from(addr);
            Ok(SocketAddr::new(std::net::IpAddr::V4(ip), port))
        }
        0x02 => {
            // IPv6
            if data.len() < 20 {
                return Err(StunError::ServerError("IPv6 address too short".into()));
            }
            Err(StunError::ServerError("IPv6 not yet supported".into()))
        }
        _ => Err(StunError::ServerError(format!(
            "unknown address family: {family}"
        ))),
    }
}

/// Parse MAPPED-ADDRESS attribute (non-XOR, for legacy servers).
fn parse_mapped_address(data: &[u8]) -> Result<SocketAddr, StunError> {
    if data.len() < 8 {
        return Err(StunError::ServerError("MAPPED-ADDRESS too short".into()));
    }

    let family = data[1];
    let port = u16::from_be_bytes([data[2], data[3]]);

    match family {
        0x01 => {
            let ip = std::net::Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            Ok(SocketAddr::new(std::net::IpAddr::V4(ip), port))
        }
        _ => Err(StunError::ServerError(format!(
            "unknown address family: {family}"
        ))),
    }
}

/// Perform a STUN binding request to discover the public address.
///
/// `server` should be a STUN server address like "stun.l.google.com:19302".
/// `timeout` is the maximum time to wait for a response.
pub async fn stun_binding_request(
    server: &str,
    timeout: Duration,
) -> Result<StunResult, StunError> {
    let server_addr: SocketAddr = tokio::net::lookup_host(server)
        .await
        .map_err(|e| StunError::ServerError(format!("DNS lookup failed: {e}")))?
        .next()
        .ok_or_else(|| StunError::ServerError("no address for STUN server".into()))?;

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local_addr = socket.local_addr()?;

    let (request, txn_id) = build_binding_request();
    let start = Instant::now();

    // Send the binding request
    socket.send_to(&request, server_addr).await?;

    // Wait for response with timeout
    let mut buf = [0u8; 1024];
    let n = tokio::time::timeout(timeout, socket.recv_from(&mut buf))
        .await
        .map_err(|_| StunError::Timeout)?
        .map_err(StunError::Io)?
        .0;

    let rtt = start.elapsed();
    let mapped_address = parse_binding_response(&buf[..n], &txn_id)?;

    // Determine NAT type based on whether mapped address matches local
    let nat_type = if mapped_address.ip() == local_addr.ip() {
        NatType::None
    } else {
        // Single request can only distinguish None vs "some NAT"
        // Full NAT type detection requires multiple servers (RFC 3489)
        NatType::Unknown
    };

    Ok(StunResult {
        mapped_address,
        local_address: local_addr,
        nat_type,
        rtt_ms: rtt.as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_is_20_bytes() {
        let (msg, txn_id) = build_binding_request();
        assert_eq!(msg.len(), 20);
        assert_eq!(txn_id.len(), 12);
        // Check magic cookie
        assert_eq!(msg[4], 0x21);
        assert_eq!(msg[5], 0x12);
        assert_eq!(msg[6], 0xA4);
        assert_eq!(msg[7], 0x42);
        // Check message type is Binding Request
        assert_eq!(msg[0], 0x00);
        assert_eq!(msg[1], 0x01);
        // Check length is 0
        assert_eq!(msg[2], 0x00);
        assert_eq!(msg[3], 0x00);
    }

    #[test]
    fn different_requests_have_different_txn_ids() {
        let (_, txn1) = build_binding_request();
        let (_, txn2) = build_binding_request();
        assert_ne!(txn1, txn2);
    }

    #[test]
    fn parse_xor_mapped_ipv4() {
        // Construct a valid XOR-MAPPED-ADDRESS for 1.2.3.4:5678
        // port 5678 = 0x162E, XOR with 0x2112 = 0x373C
        // IP 0x01020304 XOR 0x2112A442 = 0x2010A746
        let data = [
            0x00, 0x01, // reserved + family (IPv4)
            0x37, 0x3C, // XOR port (5678 ^ 0x2112)
            0x20, 0x10, 0xA7, 0x46, // XOR address
        ];
        let addr = parse_xor_mapped_address(&data).unwrap();
        assert_eq!(
            addr.ip(),
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 2, 3, 4))
        );
        assert_eq!(addr.port(), 5678);
    }

    #[test]
    fn parse_response_validates_txn_id() {
        let (request, txn_id) = build_binding_request();
        let wrong_txn = [0xFF; 12];

        // Build a minimal valid response header
        let mut response = vec![
            0x01, 0x01, // Binding Response
            0x00, 0x00, // length 0
            0x21, 0x12, 0xA4, 0x42, // magic cookie
        ];
        response.extend_from_slice(&txn_id);

        // Should fail with wrong txn_id
        let mut bad_response = response.clone();
        bad_response[8..20].copy_from_slice(&wrong_txn);
        assert!(parse_binding_response(&bad_response, &txn_id).is_err());
    }

    // Integration test: only run manually or in environments with internet access
    #[tokio::test]
    #[ignore] // requires network access to Google STUN server
    async fn stun_google() {
        let result = stun_binding_request("stun.l.google.com:19302", Duration::from_secs(5))
            .await
            .unwrap();
        println!("Mapped address: {}", result.mapped_address);
        println!("Local address: {}", result.local_address);
        println!("NAT type: {:?}", result.nat_type);
        println!("RTT: {}ms", result.rtt_ms);
        assert!(result.mapped_address.port() > 0);
    }
}
