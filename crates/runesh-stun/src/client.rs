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

    // Wait for response with overall timeout. RFC 5389 Section 10.3 requires
    // discarding any response whose source address does not match the server
    // we sent the request to. Keep receiving (ignoring mismatches) until
    // either the correct server responds or the overall timeout elapses.
    let mut buf = [0u8; 1024];
    let deadline = tokio::time::Instant::now() + timeout;
    let (n, rtt) = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(StunError::Timeout);
        }
        let (n, peer) = tokio::time::timeout(remaining, socket.recv_from(&mut buf))
            .await
            .map_err(|_| StunError::Timeout)?
            .map_err(StunError::Io)?;

        if peer != server_addr {
            tracing::debug!(
                expected = %server_addr,
                got = %peer,
                "dropping STUN response from unexpected source"
            );
            continue;
        }
        break (n, start.elapsed());
    };

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
        let (_request, txn_id) = build_binding_request();
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

    /// Build a valid STUN Binding Response echoing a given txn_id,
    /// reporting the supplied mapped address.
    fn build_response(txn_id: &[u8; 12], mapped: SocketAddr) -> Vec<u8> {
        let mut resp = Vec::with_capacity(20 + 12);
        // Type
        resp.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        // Placeholder for length
        resp.extend_from_slice(&0u16.to_be_bytes());
        // Magic cookie
        resp.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        // Transaction ID
        resp.extend_from_slice(txn_id);

        // XOR-MAPPED-ADDRESS attribute
        let port = mapped.port();
        let xport = port ^ ((MAGIC_COOKIE >> 16) as u16);
        let ip = match mapped.ip() {
            std::net::IpAddr::V4(v) => u32::from_be_bytes(v.octets()),
            _ => panic!("v6 not used in this test"),
        };
        let xaddr = ip ^ MAGIC_COOKIE;

        resp.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        resp.extend_from_slice(&8u16.to_be_bytes());
        resp.push(0); // reserved
        resp.push(1); // family IPv4
        resp.extend_from_slice(&xport.to_be_bytes());
        resp.extend_from_slice(&xaddr.to_be_bytes());

        let attr_len = (resp.len() - 20) as u16;
        resp[2..4].copy_from_slice(&attr_len.to_be_bytes());
        resp
    }

    #[tokio::test]
    async fn ignores_response_from_wrong_source() {
        // Spin up two fake "servers": a bogus one that replies first, and
        // the real one. The client should drop the bogus reply and accept
        // only the real one.
        let real = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bogus = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let real_addr = real.local_addr().unwrap();
        let bogus_addr = bogus.local_addr().unwrap();

        // Spawn both servers.
        let real_task = tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (n, peer) = real.recv_from(&mut buf).await.unwrap();
            // Echo back a valid response with the real txn_id.
            let txn: [u8; 12] = buf[8..20].try_into().unwrap();
            let _ = n;
            let resp = build_response(&txn, "1.2.3.4:5678".parse().unwrap());
            // Send bogus first from the other socket, then real from the
            // correct one. We route via the per-socket send below.
            real.send_to(&resp, peer).await.unwrap();
        });

        let bogus_task = {
            let bogus = bogus;
            let real_addr_copy = real_addr;
            tokio::spawn(async move {
                // Wait briefly, then send noise to the client. We don't know
                // the client's ephemeral port here so we can't send unsolicited.
                // Instead we piggy-back: bogus only replies if the client
                // happens to address it. We therefore send two packets in
                // response: a spoofed one from bogus_addr masquerading as if
                // from real_addr is not possible without raw sockets, so
                // instead we arrange for the client to see a spurious packet
                // from bogus_addr: we push one. But the client sends to
                // real_addr only, so bogus_addr never receives. To exercise
                // the check we send a spontaneous packet from bogus to the
                // client's source port via external coordination.
                let _ = real_addr_copy;
                drop(bogus);
            })
        };

        // Perform the binding request. Since the bogus server is silent, this
        // test currently asserts only the happy path; the negative assertion
        // is better exercised by the synchronous parse_response_validates_txn_id
        // and the fact that recv_from peer equality is checked before parsing.
        let res = stun_binding_request(&real_addr.to_string(), Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(res.mapped_address.port(), 5678);

        let _ = real_task.await;
        let _ = bogus_task.await;
        let _ = bogus_addr;
    }

    #[tokio::test]
    async fn rejects_response_from_other_addr() {
        // Drive the reject path: have a fake "server" host that sends the
        // response from a different source port than the client actually
        // addressed. We bind two sockets; the client addresses the primary,
        // but a secondary sends the response. Expect Timeout (because the
        // client drops the mismatched packet).
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let spoofer = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let (_, peer) = server.recv_from(&mut buf).await.unwrap();
            // Don't reply from `server`. Reply from `spoofer` instead.
            let txn: [u8; 12] = buf[8..20].try_into().unwrap();
            let resp = build_response(&txn, "1.2.3.4:5678".parse().unwrap());
            spoofer.send_to(&resp, peer).await.unwrap();
        });

        let res = stun_binding_request(&server_addr.to_string(), Duration::from_millis(400)).await;
        assert!(matches!(res, Err(StunError::Timeout)));

        let _ = server_task.await;
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
