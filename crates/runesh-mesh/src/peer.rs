//! Peer map management.
//!
//! Tracks all peers in a tenant's mesh: their public keys, endpoints,
//! allowed IPs, DERP relay preferences, and connection state.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::keys::WgPublicKey;

/// Information about a single mesh peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// The peer's WireGuard public key.
    pub public_key: WgPublicKey,

    /// The peer's mesh IP address.
    pub mesh_ip: Ipv4Addr,

    /// Known UDP endpoints for direct connection.
    #[serde(default)]
    pub endpoints: Vec<SocketAddr>,

    /// CIDRs this peer is allowed to route (subnet router).
    #[serde(default)]
    pub allowed_ips: Vec<IpNet>,

    /// Preferred DERP relay region ID.
    #[serde(default)]
    pub derp_region: Option<u16>,

    /// Whether this peer is an exit node.
    #[serde(default)]
    pub is_exit_node: bool,

    /// Whether this peer is a subnet router.
    #[serde(default)]
    pub is_subnet_router: bool,

    /// Hostname for MagicDNS.
    #[serde(default)]
    pub hostname: Option<String>,

    /// Operating system.
    #[serde(default)]
    pub os: Option<String>,

    /// Tags assigned to this peer (for ACL matching).
    #[serde(default)]
    pub tags: Vec<String>,

    /// User identity that owns this peer.
    #[serde(default)]
    pub user: Option<String>,

    /// Whether this peer is currently online.
    #[serde(default)]
    pub online: bool,

    /// Last seen timestamp (serialized as epoch seconds).
    #[serde(default)]
    pub last_seen: Option<u64>,
}

/// A map of all peers in a tenant's mesh.
#[derive(Debug, Clone, Default)]
pub struct PeerMap {
    /// Peers indexed by public key.
    peers: HashMap<WgPublicKey, PeerInfo>,
    /// Reverse lookup: mesh IP to public key.
    ip_to_key: HashMap<Ipv4Addr, WgPublicKey>,
}

impl PeerMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or update a peer.
    pub fn upsert(&mut self, peer: PeerInfo) {
        let key = peer.public_key.clone();
        let ip = peer.mesh_ip;

        // Remove old IP mapping if the key already exists with a different IP
        if let Some(existing) = self.peers.get(&key)
            && existing.mesh_ip != ip
        {
            self.ip_to_key.remove(&existing.mesh_ip);
        }

        self.ip_to_key.insert(ip, key.clone());
        self.peers.insert(key, peer);
    }

    /// Remove a peer by public key.
    pub fn remove(&mut self, key: &WgPublicKey) -> Option<PeerInfo> {
        if let Some(peer) = self.peers.remove(key) {
            self.ip_to_key.remove(&peer.mesh_ip);
            Some(peer)
        } else {
            None
        }
    }

    /// Get a peer by public key.
    pub fn get(&self, key: &WgPublicKey) -> Option<&PeerInfo> {
        self.peers.get(key)
    }

    /// Get a peer by mesh IP.
    pub fn get_by_ip(&self, ip: Ipv4Addr) -> Option<&PeerInfo> {
        self.ip_to_key.get(&ip).and_then(|key| self.peers.get(key))
    }

    /// Find the peer that should handle a destination IP.
    ///
    /// Checks mesh IPs first, then allowed_ips (subnet routes).
    /// Returns the peer's public key if found.
    pub fn route(&self, dst_ip: Ipv4Addr) -> Option<&WgPublicKey> {
        // Direct mesh IP match
        if let Some(key) = self.ip_to_key.get(&dst_ip) {
            return Some(key);
        }

        // Subnet route match (most specific wins)
        let dst = std::net::IpAddr::V4(dst_ip);
        let mut best_match: Option<(&WgPublicKey, u8)> = None;

        for (key, peer) in &self.peers {
            for allowed in &peer.allowed_ips {
                if allowed.contains(&dst) {
                    let prefix = allowed.prefix_len();
                    if best_match.is_none() || prefix > best_match.unwrap().1 {
                        best_match = Some((key, prefix));
                    }
                }
            }
        }

        best_match.map(|(key, _)| key)
    }

    /// Iterate over all peers.
    pub fn iter(&self) -> impl Iterator<Item = (&WgPublicKey, &PeerInfo)> {
        self.peers.iter()
    }

    /// Number of peers.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Get all online peers.
    pub fn online_peers(&self) -> Vec<&PeerInfo> {
        self.peers.values().filter(|p| p.online).collect()
    }

    /// Mark a peer as online/offline.
    pub fn set_online(&mut self, key: &WgPublicKey, online: bool) {
        if let Some(peer) = self.peers.get_mut(key) {
            peer.online = online;
            if online {
                peer.last_seen = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
        }
    }

    /// Update a peer's endpoints.
    pub fn update_endpoints(&mut self, key: &WgPublicKey, endpoints: Vec<SocketAddr>) {
        if let Some(peer) = self.peers.get_mut(key) {
            peer.endpoints = endpoints;
        }
    }

    /// Serialize the peer map for distribution to clients.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let peers: Vec<&PeerInfo> = self.peers.values().collect();
        serde_json::to_string(&peers)
    }
}

/// A network map response sent to a peer.
///
/// Contains only the peers that this specific node should know about
/// (filtered by ACLs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetMap {
    /// This node's mesh IP.
    pub self_ip: Ipv4Addr,
    /// Peers this node can communicate with.
    pub peers: Vec<PeerInfo>,
    /// DERP relay map.
    #[serde(default)]
    pub derp_map: Option<DerpMap>,
    /// DNS configuration.
    #[serde(default)]
    pub dns: Option<DnsConfig>,
}

/// DERP relay server information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerpRegion {
    pub region_id: u16,
    pub region_code: String,
    pub region_name: String,
    pub nodes: Vec<DerpNode>,
}

/// A single DERP relay node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerpNode {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub stun_port: u16,
}

/// DERP relay map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerpMap {
    pub regions: HashMap<u16, DerpRegion>,
}

/// DNS configuration for MagicDNS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    /// DNS search domain (e.g., "tenant.mesh.local").
    pub domain: String,
    /// Nameserver addresses.
    pub nameservers: Vec<Ipv4Addr>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::WgKeypair;

    fn make_peer(name: &str, ip: Ipv4Addr) -> PeerInfo {
        let kp = WgKeypair::generate();
        PeerInfo {
            public_key: WgPublicKey::from_public(&kp.public),
            mesh_ip: ip,
            endpoints: vec![],
            allowed_ips: vec![],
            derp_region: None,
            is_exit_node: false,
            is_subnet_router: false,
            hostname: Some(name.to_string()),
            os: None,
            tags: vec![],
            user: None,
            online: true,
            last_seen: None,
        }
    }

    #[test]
    fn add_and_lookup() {
        let mut map = PeerMap::new();
        let peer = make_peer("server1", Ipv4Addr::new(100, 64, 0, 1));
        let key = peer.public_key.clone();
        map.upsert(peer);

        assert_eq!(map.len(), 1);
        assert!(map.get(&key).is_some());
        assert!(map.get_by_ip(Ipv4Addr::new(100, 64, 0, 1)).is_some());
    }

    #[test]
    fn remove_peer() {
        let mut map = PeerMap::new();
        let peer = make_peer("server1", Ipv4Addr::new(100, 64, 0, 1));
        let key = peer.public_key.clone();
        map.upsert(peer);
        map.remove(&key);

        assert_eq!(map.len(), 0);
        assert!(map.get_by_ip(Ipv4Addr::new(100, 64, 0, 1)).is_none());
    }

    #[test]
    fn route_direct_ip() {
        let mut map = PeerMap::new();
        let peer = make_peer("server1", Ipv4Addr::new(100, 64, 0, 1));
        let key = peer.public_key.clone();
        map.upsert(peer);

        assert_eq!(map.route(Ipv4Addr::new(100, 64, 0, 1)), Some(&key));
        assert_eq!(map.route(Ipv4Addr::new(100, 64, 0, 2)), None);
    }

    #[test]
    fn route_via_subnet_router() {
        let mut map = PeerMap::new();
        let mut peer = make_peer("router", Ipv4Addr::new(100, 64, 0, 1));
        peer.allowed_ips = vec!["192.168.1.0/24".parse().unwrap()];
        peer.is_subnet_router = true;
        let key = peer.public_key.clone();
        map.upsert(peer);

        assert_eq!(map.route(Ipv4Addr::new(192, 168, 1, 50)), Some(&key));
        assert_eq!(map.route(Ipv4Addr::new(192, 168, 2, 50)), None);
    }

    #[test]
    fn most_specific_route_wins() {
        let mut map = PeerMap::new();

        let mut broad = make_peer("broad", Ipv4Addr::new(100, 64, 0, 1));
        broad.allowed_ips = vec!["10.0.0.0/8".parse().unwrap()];
        let broad_key = broad.public_key.clone();
        map.upsert(broad);

        let mut specific = make_peer("specific", Ipv4Addr::new(100, 64, 0, 2));
        specific.allowed_ips = vec!["10.1.0.0/16".parse().unwrap()];
        let specific_key = specific.public_key.clone();
        map.upsert(specific);

        // 10.1.0.5 matches both, but /16 is more specific
        assert_eq!(map.route(Ipv4Addr::new(10, 1, 0, 5)), Some(&specific_key));
        // 10.2.0.5 only matches /8
        assert_eq!(map.route(Ipv4Addr::new(10, 2, 0, 5)), Some(&broad_key));
    }

    #[test]
    fn online_tracking() {
        let mut map = PeerMap::new();
        let peer = make_peer("server1", Ipv4Addr::new(100, 64, 0, 1));
        let key = peer.public_key.clone();
        map.upsert(peer);

        assert_eq!(map.online_peers().len(), 1);
        map.set_online(&key, false);
        assert_eq!(map.online_peers().len(), 0);
    }

    #[test]
    fn json_roundtrip() {
        let mut map = PeerMap::new();
        map.upsert(make_peer("a", Ipv4Addr::new(100, 64, 0, 1)));
        map.upsert(make_peer("b", Ipv4Addr::new(100, 64, 0, 2)));

        let json = map.to_json().unwrap();
        let peers: Vec<PeerInfo> = serde_json::from_str(&json).unwrap();
        assert_eq!(peers.len(), 2);
    }
}
