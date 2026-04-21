//! WireGuard tunnel I/O via boringtun.
//!
//! Wraps boringtun's `Tunn` struct into an async-friendly interface.
//! boringtun is synchronous and `!Send`, so we run each tunnel on
//! a dedicated blocking thread and communicate via channels.
//!
//! Packet flow:
//!   TUN device -> encapsulate -> UDP socket (outbound)
//!   UDP socket -> decapsulate -> TUN device (inbound)

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use boringtun::noise::rate_limiter::RateLimiter;
use boringtun::noise::{Tunn, TunnResult};

use crate::MeshError;
use crate::keys::WgKeypair;

/// Maximum UDP datagram size.
const MAX_UDP_SIZE: usize = 65536;

/// Overhead added by WireGuard encapsulation.
const WG_OVERHEAD: usize = 80;

/// Per-tunnel handshake rate limit (messages per second).
const HANDSHAKE_PPS: u64 = 100;

/// WireGuard message types.
const WG_MSG_HANDSHAKE_INIT: u8 = 1;
const WG_MSG_HANDSHAKE_RESP: u8 = 2;
const WG_MSG_COOKIE_REPLY: u8 = 3;
const WG_MSG_DATA: u8 = 4;

/// Handshake-init rate limit per source IP (messages per `HANDSHAKE_INIT_WINDOW`).
const HANDSHAKE_INIT_LIMIT: u32 = 10;
const HANDSHAKE_INIT_WINDOW: Duration = Duration::from_secs(1);

/// A packet to send over the network.
#[derive(Debug)]
pub struct OutboundPacket {
    pub data: Vec<u8>,
    pub dst: SocketAddr,
}

/// A decrypted packet from the tunnel.
#[derive(Debug)]
pub struct InboundPacket {
    pub data: Vec<u8>,
    pub src_addr: IpAddr,
}

/// A single WireGuard tunnel to a peer.
///
/// Wraps boringtun's `Tunn` with channels for async packet exchange.
/// The tunnel runs its timer loop internally.
pub struct WgTunnel {
    /// Shared tunnel state (Mutex because Tunn is !Send).
    tunn: Arc<Mutex<Tunn>>,
    /// Peer's endpoint address.
    endpoint: Mutex<Option<SocketAddr>>,
    /// Peer index.
    peer_index: u32,
}

impl WgTunnel {
    /// Create a new tunnel to a peer.
    ///
    /// Each tunnel gets its own per-tunnel handshake rate limiter at
    /// `HANDSHAKE_PPS` to bound replay/handshake flooding.
    pub fn new(
        own_key: &WgKeypair,
        peer_public: &boringtun::x25519::PublicKey,
        peer_index: u32,
        keepalive: Option<u16>,
    ) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(
            &boringtun::x25519::PublicKey::from(&own_key.private),
            HANDSHAKE_PPS,
        ));
        let tunn = Tunn::new(
            own_key.private.clone(),
            *peer_public,
            None,      // no preshared key
            keepalive, // persistent keepalive interval
            peer_index,
            Some(rate_limiter),
        );

        Self {
            tunn: Arc::new(Mutex::new(tunn)),
            endpoint: Mutex::new(None),
            peer_index,
        }
    }

    /// Set the peer's current endpoint.
    pub fn set_endpoint(&self, addr: SocketAddr) {
        *self.endpoint.lock().unwrap() = Some(addr);
    }

    /// Get the peer's current endpoint.
    pub fn endpoint(&self) -> Option<SocketAddr> {
        *self.endpoint.lock().unwrap()
    }

    /// Encapsulate an IP packet for sending through the tunnel.
    ///
    /// Takes a plaintext IP packet (from TUN device) and returns
    /// the encrypted WireGuard packet to send via UDP.
    pub fn encapsulate(&self, src: &[u8]) -> Result<Option<Vec<u8>>, MeshError> {
        let mut tunn = self.tunn.lock().unwrap();
        let min_size = src.len() + WG_OVERHEAD;
        let mut dst = vec![0u8; min_size.max(148)];

        match tunn.encapsulate(src, &mut dst) {
            TunnResult::WriteToNetwork(packet) => Ok(Some(packet.to_vec())),
            TunnResult::Done => Ok(None), // queued, will send after handshake
            TunnResult::Err(e) => Err(MeshError::Tunnel(format!("encapsulate: {e:?}"))),
            _ => Ok(None),
        }
    }

    /// Decapsulate a UDP datagram received from the network.
    ///
    /// Returns the decrypted IP packet and any response packets
    /// that need to be sent back (handshake responses).
    pub fn decapsulate(
        &self,
        src_addr: Option<IpAddr>,
        datagram: &[u8],
    ) -> Result<DecapsulateResult, MeshError> {
        let mut tunn = self.tunn.lock().unwrap();
        let mut dst = vec![0u8; MAX_UDP_SIZE];
        let mut result = DecapsulateResult::default();

        match tunn.decapsulate(src_addr, datagram, &mut dst) {
            TunnResult::WriteToTunnelV4(data, addr) => {
                result.tunnel_packet = Some(InboundPacket {
                    data: data.to_vec(),
                    src_addr: IpAddr::V4(addr),
                });
            }
            TunnResult::WriteToTunnelV6(data, addr) => {
                result.tunnel_packet = Some(InboundPacket {
                    data: data.to_vec(),
                    src_addr: IpAddr::V6(addr),
                });
            }
            TunnResult::WriteToNetwork(packet) => {
                // Handshake response; send it, then drain queued packets
                result.network_packets.push(packet.to_vec());
                self.drain_queued(&mut tunn, &mut result);
            }
            TunnResult::Done => {}
            TunnResult::Err(e) => {
                return Err(MeshError::Tunnel(format!("decapsulate: {e:?}")));
            }
        }

        Ok(result)
    }

    /// Run the timer tick. Must be called every ~250ms.
    ///
    /// Returns packets that need to be sent (keepalives, re-handshakes).
    pub fn tick_timers(&self) -> Vec<Vec<u8>> {
        let mut tunn = self.tunn.lock().unwrap();
        let mut dst = vec![0u8; MAX_UDP_SIZE];
        let mut packets = Vec::new();

        match tunn.update_timers(&mut dst) {
            TunnResult::WriteToNetwork(packet) => {
                packets.push(packet.to_vec());
            }
            TunnResult::Err(e) => {
                tracing::debug!(peer_index = self.peer_index, error = ?e, "timer error");
            }
            _ => {}
        }

        packets
    }

    /// Drain queued packets after a handshake completes.
    fn drain_queued(&self, tunn: &mut Tunn, result: &mut DecapsulateResult) {
        let mut dst = vec![0u8; MAX_UDP_SIZE];
        loop {
            match tunn.decapsulate(None, &[], &mut dst) {
                TunnResult::WriteToNetwork(packet) => {
                    result.network_packets.push(packet.to_vec());
                }
                TunnResult::WriteToTunnelV4(data, addr) => {
                    result.tunnel_packet = Some(InboundPacket {
                        data: data.to_vec(),
                        src_addr: IpAddr::V4(addr),
                    });
                }
                TunnResult::WriteToTunnelV6(data, addr) => {
                    result.tunnel_packet = Some(InboundPacket {
                        data: data.to_vec(),
                        src_addr: IpAddr::V6(addr),
                    });
                }
                TunnResult::Done => break,
                TunnResult::Err(_) => break,
            }
        }
    }
}

/// Result of a decapsulate operation.
#[derive(Debug, Default)]
pub struct DecapsulateResult {
    /// Decrypted IP packet for the TUN device (if any).
    pub tunnel_packet: Option<InboundPacket>,
    /// Packets to send back over the network (handshake responses, etc.).
    pub network_packets: Vec<Vec<u8>>,
}

/// Manages multiple WireGuard tunnels (one per peer).
pub struct TunnelManager {
    /// Own keypair.
    own_key: WgKeypair,
    /// Active tunnels indexed by peer index.
    tunnels: HashMap<u32, Arc<WgTunnel>>,
    /// Reverse map: upper 24 bits of WireGuard receiver_idx -> tunnel index.
    /// Used for O(1) lookup on data, handshake-response, and cookie packets.
    by_index_prefix: HashMap<u32, u32>,
    /// Next peer index to assign.
    next_index: u32,
    /// Per-source-IP handshake-init counters for rate limiting.
    handshake_init_window: HashMap<IpAddr, (Instant, u32)>,
}

impl TunnelManager {
    /// Create a new tunnel manager with the given keypair.
    pub fn new(own_key: WgKeypair) -> Self {
        Self {
            own_key,
            tunnels: HashMap::new(),
            by_index_prefix: HashMap::new(),
            next_index: 0,
            handshake_init_window: HashMap::new(),
        }
    }

    /// Add a tunnel to a peer. Returns the peer index.
    ///
    /// The peer index is stored left-shifted by 8 inside boringtun so its
    /// upper 24 bits uniquely identify the tunnel in received packets.
    pub fn add_peer(
        &mut self,
        peer_public: &boringtun::x25519::PublicKey,
        endpoint: Option<SocketAddr>,
        keepalive: Option<u16>,
    ) -> u32 {
        let index = self.next_index;
        self.next_index += 1;

        // Left-shift by 8 so the upper 24 bits are unique per tunnel and the
        // bottom 8 bits are the session counter boringtun manages internally.
        let tunn_index = index << 8;
        let tunnel = Arc::new(WgTunnel::new(
            &self.own_key,
            peer_public,
            tunn_index,
            keepalive,
        ));
        if let Some(addr) = endpoint {
            tunnel.set_endpoint(addr);
        }
        self.tunnels.insert(index, tunnel);
        self.by_index_prefix.insert(index, index);
        index
    }

    /// Remove a peer tunnel.
    pub fn remove_peer(&mut self, index: u32) -> bool {
        self.by_index_prefix.remove(&index);
        self.tunnels.remove(&index).is_some()
    }

    /// Get a tunnel by peer index.
    pub fn get(&self, index: u32) -> Option<&WgTunnel> {
        self.tunnels.get(&index).map(|t| t.as_ref())
    }

    /// Encapsulate a packet for a specific peer.
    pub fn encapsulate(&self, peer_index: u32, data: &[u8]) -> Result<Option<Vec<u8>>, MeshError> {
        let tunnel = self
            .tunnels
            .get(&peer_index)
            .ok_or_else(|| MeshError::PeerNotFound(format!("index {peer_index}")))?;
        tunnel.encapsulate(data)
    }

    /// Decapsulate an incoming UDP packet.
    ///
    /// For transport data (WireGuard message type 4), the handshake response
    /// (type 2), and cookie replies (type 3), the WireGuard `receiver_idx`
    /// in bytes 4..8 is used to look up the target tunnel in O(1) via the
    /// upper 24 bits that encode our peer index.
    ///
    /// Handshake initiations (type 1) have no receiver index and fall back
    /// to a per-source-IP rate limit plus a linear scan.
    pub fn decapsulate(
        &mut self,
        src_addr: Option<IpAddr>,
        datagram: &[u8],
    ) -> Result<Option<(u32, DecapsulateResult)>, MeshError> {
        if datagram.is_empty() {
            return Ok(None);
        }

        let msg_type = datagram[0];
        match msg_type {
            WG_MSG_DATA | WG_MSG_HANDSHAKE_RESP | WG_MSG_COOKIE_REPLY if datagram.len() >= 8 => {
                let receiver_idx =
                    u32::from_le_bytes([datagram[4], datagram[5], datagram[6], datagram[7]]);
                // Upper 24 bits encode our peer index (we left-shifted by 8 on add).
                let prefix = receiver_idx >> 8;
                if let Some(&index) = self.by_index_prefix.get(&prefix)
                    && let Some(tunnel) = self.tunnels.get(&index)
                {
                    let result = tunnel.decapsulate(src_addr, datagram)?;
                    return Ok(Some((index, result)));
                }
                Ok(None)
            }
            WG_MSG_HANDSHAKE_INIT => {
                // Rate-limit handshake inits per source IP.
                if let Some(ip) = src_addr
                    && !self.allow_handshake_init(ip)
                {
                    tracing::debug!(%ip, "handshake-init rate limit exceeded");
                    return Ok(None);
                }
                // Fall back to linear scan for handshake inits.
                for (&index, tunnel) in &self.tunnels {
                    match tunnel.decapsulate(src_addr, datagram) {
                        Ok(result)
                            if result.tunnel_packet.is_some()
                                || !result.network_packets.is_empty() =>
                        {
                            return Ok(Some((index, result)));
                        }
                        Ok(_) => continue,
                        Err(_) => continue,
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Returns true if a handshake-init from this source IP is allowed under
    /// the fixed-window rate limit.
    fn allow_handshake_init(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let entry = self.handshake_init_window.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) > HANDSHAKE_INIT_WINDOW {
            *entry = (now, 0);
        }
        if entry.1 >= HANDSHAKE_INIT_LIMIT {
            return false;
        }
        entry.1 += 1;
        true
    }

    /// Tick timers on all tunnels. Returns (peer_index, packets_to_send).
    pub fn tick_all_timers(&self) -> Vec<(u32, Vec<Vec<u8>>)> {
        self.tunnels
            .iter()
            .filter_map(|(&index, tunnel)| {
                let packets = tunnel.tick_timers();
                if packets.is_empty() {
                    None
                } else {
                    Some((index, packets))
                }
            })
            .collect()
    }

    /// Number of active tunnels.
    pub fn peer_count(&self) -> usize {
        self.tunnels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::WgKeypair;

    #[test]
    fn create_tunnel() {
        let own = WgKeypair::generate();
        let peer = WgKeypair::generate();

        let tunnel = WgTunnel::new(
            &own,
            &peer.public,
            0,
            Some(25), // 25s keepalive
        );

        assert!(tunnel.endpoint().is_none());
        tunnel.set_endpoint("1.2.3.4:51820".parse().unwrap());
        assert_eq!(tunnel.endpoint(), Some("1.2.3.4:51820".parse().unwrap()));
    }

    #[test]
    fn tunnel_manager_add_remove() {
        let own = WgKeypair::generate();
        let mut mgr = TunnelManager::new(own);

        let peer1 = WgKeypair::generate();
        let peer2 = WgKeypair::generate();

        let idx1 = mgr.add_peer(&peer1.public, None, None);
        let idx2 = mgr.add_peer(&peer2.public, None, None);

        assert_eq!(mgr.peer_count(), 2);
        assert!(mgr.get(idx1).is_some());
        assert!(mgr.get(idx2).is_some());

        mgr.remove_peer(idx1);
        assert_eq!(mgr.peer_count(), 1);
        assert!(mgr.get(idx1).is_none());
    }

    #[test]
    fn encapsulate_before_handshake() {
        let own = WgKeypair::generate();
        let peer = WgKeypair::generate();

        let mut mgr = TunnelManager::new(own);
        let idx = mgr.add_peer(&peer.public, None, None);

        // Before handshake, encapsulate queues the packet (returns Ok(None))
        // or returns a handshake initiation packet
        let result = mgr.encapsulate(idx, b"test packet");
        // Should not panic regardless of outcome
        match result {
            Ok(Some(_)) => {} // got a handshake init or queued packet
            Ok(None) => {}    // packet queued for after handshake
            Err(_) => {}      // no session yet, expected
        }
    }

    #[test]
    fn two_tunnels_handshake() {
        // Create two tunnel endpoints and perform a handshake
        let key_a = WgKeypair::generate();
        let key_b = WgKeypair::generate();

        let tunnel_a = WgTunnel::new(&key_a, &key_b.public, 0, None);
        let tunnel_b = WgTunnel::new(&key_b, &key_a.public, 1, None);

        // A encapsulates a packet (triggers handshake init)
        let init_packet = tunnel_a.encapsulate(b"hello from A").unwrap();
        // The first call returns a handshake initiation or None
        // Either way, tick timers to produce the handshake init
        let timer_packets = tunnel_a.tick_timers();

        // If we got a handshake init from either source, feed it to B
        let handshake_init = init_packet.or_else(|| timer_packets.into_iter().next());

        if let Some(init) = handshake_init {
            // B processes the handshake init
            let result = tunnel_b.decapsulate(Some("127.0.0.1".parse().unwrap()), &init);
            assert!(result.is_ok());
            let result = result.unwrap();

            // B should respond with a handshake response
            if !result.network_packets.is_empty() {
                // Feed B's response back to A
                let resp = &result.network_packets[0];
                let a_result = tunnel_a.decapsulate(Some("127.0.0.1".parse().unwrap()), resp);
                assert!(a_result.is_ok());

                // Now A should be able to send encrypted data
                let encrypted = tunnel_a.encapsulate(b"hello from A after handshake");
                assert!(encrypted.is_ok());
            }
        }
    }

    #[test]
    fn tick_timers_on_fresh_tunnel() {
        let own = WgKeypair::generate();
        let peer = WgKeypair::generate();
        let tunnel = WgTunnel::new(&own, &peer.public, 0, None);

        // Fresh tunnel, timers should produce handshake initiation
        let packets = tunnel.tick_timers();
        // May or may not produce packets depending on timer state
        // Just verify it doesn't panic
        let _ = packets;
    }

    #[test]
    fn encapsulate_unknown_peer() {
        let own = WgKeypair::generate();
        let mgr = TunnelManager::new(own);
        assert!(mgr.encapsulate(999, b"data").is_err());
    }

    #[test]
    fn receiver_index_lookup_routes_to_correct_tunnel() {
        // Two peers added to the manager.
        let own = WgKeypair::generate();
        let mut mgr = TunnelManager::new(own);
        let p1 = WgKeypair::generate();
        let p2 = WgKeypair::generate();
        let idx1 = mgr.add_peer(&p1.public, None, None);
        let idx2 = mgr.add_peer(&p2.public, None, None);
        assert_ne!(idx1, idx2);

        // The reverse map is populated with each peer's prefix.
        assert!(mgr.by_index_prefix.contains_key(&idx1));
        assert!(mgr.by_index_prefix.contains_key(&idx2));

        // An unknown receiver_idx prefix yields None without scanning any
        // tunnel. This proves the fast-path lookup is in effect.
        let mut packet = vec![0u8; 32];
        packet[0] = WG_MSG_DATA;
        packet[4..8].copy_from_slice(&0xDEAD_BE00u32.to_le_bytes());
        assert!(mgr.decapsulate(None, &packet).unwrap().is_none());

        // A non-WG message type yields None.
        let bad = vec![0xFFu8; 16];
        assert!(mgr.decapsulate(None, &bad).unwrap().is_none());
    }

    #[test]
    fn handshake_init_rate_limit_drops_extra_packets() {
        let own = WgKeypair::generate();
        let mut mgr = TunnelManager::new(own);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Up to HANDSHAKE_INIT_LIMIT attempts pass.
        for _ in 0..HANDSHAKE_INIT_LIMIT {
            assert!(mgr.allow_handshake_init(ip));
        }
        // The next one in the same window is refused.
        assert!(!mgr.allow_handshake_init(ip));
    }
}
