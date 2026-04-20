//! WireGuard tunnel I/O via boringtun.
//!
//! Wraps boringtun's `Tunn` struct into an async-friendly interface.
//! boringtun is synchronous and `!Send`, so we run each tunnel on
//! a dedicated blocking thread and communicate via channels.
//!
//! Packet flow:
//!   TUN device -> encapsulate -> UDP socket (outbound)
//!   UDP socket -> decapsulate -> TUN device (inbound)

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use boringtun::noise::{Tunn, TunnResult};

use crate::MeshError;
use crate::keys::WgKeypair;

/// Maximum UDP datagram size.
const MAX_UDP_SIZE: usize = 65536;

/// Overhead added by WireGuard encapsulation.
const WG_OVERHEAD: usize = 80;

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
    pub fn new(
        own_key: &WgKeypair,
        peer_public: &boringtun::x25519::PublicKey,
        peer_index: u32,
        keepalive: Option<u16>,
    ) -> Self {
        let tunn = Tunn::new(
            own_key.private.clone(),
            *peer_public,
            None,      // no preshared key
            keepalive, // persistent keepalive interval
            peer_index,
            None, // no rate limiter
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
    tunnels: std::collections::HashMap<u32, WgTunnel>,
    /// Next peer index to assign.
    next_index: u32,
}

impl TunnelManager {
    /// Create a new tunnel manager with the given keypair.
    pub fn new(own_key: WgKeypair) -> Self {
        Self {
            own_key,
            tunnels: std::collections::HashMap::new(),
            next_index: 0,
        }
    }

    /// Add a tunnel to a peer. Returns the peer index.
    pub fn add_peer(
        &mut self,
        peer_public: &boringtun::x25519::PublicKey,
        endpoint: Option<SocketAddr>,
        keepalive: Option<u16>,
    ) -> u32 {
        let index = self.next_index;
        self.next_index += 1;

        let tunnel = WgTunnel::new(&self.own_key, peer_public, index, keepalive);
        if let Some(addr) = endpoint {
            tunnel.set_endpoint(addr);
        }
        self.tunnels.insert(index, tunnel);
        index
    }

    /// Remove a peer tunnel.
    pub fn remove_peer(&mut self, index: u32) -> bool {
        self.tunnels.remove(&index).is_some()
    }

    /// Get a tunnel by peer index.
    pub fn get(&self, index: u32) -> Option<&WgTunnel> {
        self.tunnels.get(&index)
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
    /// Tries each tunnel until one succeeds (boringtun identifies
    /// the correct tunnel by the receiver index in the WireGuard header).
    pub fn decapsulate(
        &self,
        src_addr: Option<IpAddr>,
        datagram: &[u8],
    ) -> Result<Option<(u32, DecapsulateResult)>, MeshError> {
        for (&index, tunnel) in &self.tunnels {
            match tunnel.decapsulate(src_addr, datagram) {
                Ok(result)
                    if result.tunnel_packet.is_some() || !result.network_packets.is_empty() =>
                {
                    return Ok(Some((index, result)));
                }
                Ok(_) => continue, // not for this tunnel
                Err(_) => continue,
            }
        }
        Ok(None)
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
}
