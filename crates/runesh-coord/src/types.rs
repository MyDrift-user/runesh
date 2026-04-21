//! Coordination protocol types.
//!
//! These are the data structures exchanged between the coordination server
//! and Tailscale-compatible clients. They map to Tailscale's `tailcfg` types.

use std::collections::HashMap;
// std::net types used by consumers but not directly here

use runesh_acl::AclPolicy;
use serde::{Deserialize, Serialize};

use crate::error::CoordError;

/// A registered node (machine) in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// Unique node ID (server-assigned).
    pub id: u64,

    /// Stable node ID (persists across key rotations).
    #[serde(default)]
    pub stable_id: String,

    /// Display name for this node.
    pub name: String,

    /// The node's current WireGuard public key (base64).
    pub key: String,

    /// The node's machine key (base64, used for control plane auth).
    pub machine_key: String,

    /// Mesh IP addresses assigned to this node.
    pub addresses: Vec<String>,

    /// CIDRs this node is allowed to reach (from ACLs).
    #[serde(default)]
    pub allowed_ips: Vec<String>,

    /// Known endpoints for direct WireGuard connections.
    #[serde(default)]
    pub endpoints: Vec<String>,

    /// Preferred DERP relay region.
    #[serde(default)]
    pub derp: Option<String>,

    /// Hostname reported by the node.
    #[serde(default)]
    pub hostname: String,

    /// Operating system.
    #[serde(default)]
    pub os: String,

    /// Tags assigned to this node.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Whether this node is online.
    #[serde(default)]
    pub online: bool,

    /// Last seen timestamp (RFC 3339).
    #[serde(default)]
    pub last_seen: Option<String>,

    /// User who owns this node.
    #[serde(default)]
    pub user: Option<u64>,

    /// Whether this node is authorized to join the mesh.
    #[serde(default)]
    pub authorized: bool,

    /// Creation timestamp.
    #[serde(default)]
    pub created: Option<String>,

    /// Key expiry timestamp (empty = no expiry).
    #[serde(default)]
    pub key_expiry: Option<String>,
}

/// A user in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: u64,
    pub login_name: String,
    pub display_name: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// A pre-authentication key for unattended node enrollment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreAuthKey {
    /// The key string (shown once at creation).
    pub key: String,
    /// Tenant this key belongs to.
    pub tenant_id: String,
    /// User who created this key.
    pub user: String,
    /// Whether the key can be used multiple times.
    pub reusable: bool,
    /// Whether nodes enrolled with this key are auto-authorized.
    pub ephemeral: bool,
    /// Tags automatically applied to nodes enrolled with this key.
    #[serde(default)]
    pub acl_tags: Vec<String>,
    /// Expiration timestamp.
    pub expiration: String,
    /// Whether this key has been used (for single-use keys).
    #[serde(default)]
    pub used: bool,
}

/// Registration request from a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    /// Node's WireGuard public key (base64).
    pub node_key: String,
    /// Node's machine key (base64).
    pub machine_key: String,
    /// Hostname.
    pub hostname: String,
    /// Operating system.
    #[serde(default)]
    pub os: String,
    /// Pre-auth key (for unattended enrollment).
    #[serde(default)]
    pub auth_key: Option<String>,
    /// Requested tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Known endpoints.
    #[serde(default)]
    pub endpoints: Vec<String>,
}

impl RegisterRequest {
    /// Validate that every requested tag is owned by `identity` (or by any
    /// group that `identity` belongs to) according to the ACL's `tagOwners`
    /// map. Returns `CoordError::UnauthorizedTag` for the first offending tag.
    ///
    /// `identity` should be the enrolling user's login (e.g., email). If a
    /// pre-auth key carries the identity, use its owner here.
    ///
    /// `identity_groups` is the list of `group:` names that `identity`
    /// belongs to. Pass an empty slice when groups aren't tracked.
    ///
    /// Tags that have no entry in `tagOwners` are rejected (consistent with
    /// Tailscale's behavior where a tag not in tagOwners is unassignable).
    pub fn validate_tags(
        &self,
        policy: &AclPolicy,
        identity: &str,
        identity_groups: &[String],
    ) -> Result<(), CoordError> {
        for tag in &self.tags {
            if !tag.starts_with("tag:") {
                return Err(CoordError::UnauthorizedTag(tag.clone()));
            }
            let owners = match policy.tag_owners.get(tag) {
                Some(o) => o,
                None => return Err(CoordError::UnauthorizedTag(tag.clone())),
            };
            let allowed = owners.iter().any(|owner| {
                if owner == "*" {
                    return true;
                }
                if owner == identity {
                    return true;
                }
                if owner.starts_with("group:") && identity_groups.iter().any(|g| g == owner) {
                    return true;
                }
                // Resolve nested groups declared in the policy.
                if owner.starts_with("group:")
                    && let Ok(members) = policy.resolve_group(owner)
                {
                    return members.iter().any(|m| m == identity);
                }
                false
            });
            if !allowed {
                return Err(CoordError::UnauthorizedTag(tag.clone()));
            }
        }
        Ok(())
    }
}

/// Registration response from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResponse {
    /// Whether registration succeeded.
    pub authorized: bool,
    /// Assigned node ID.
    #[serde(default)]
    pub node_id: Option<u64>,
    /// Assigned mesh IP.
    #[serde(default)]
    pub mesh_ip: Option<String>,
    /// Error message if not authorized.
    #[serde(default)]
    pub error: Option<String>,
    /// URL for interactive auth (if no auth_key provided).
    #[serde(default)]
    pub auth_url: Option<String>,
}

/// A map response sent to a node describing its view of the mesh.
///
/// This is the core data structure the coordination server pushes
/// to each node. It contains only the peers that node is allowed
/// to communicate with (filtered by ACLs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapResponse {
    /// The node itself (with its assigned IPs, etc.).
    #[serde(default)]
    pub node: Option<Node>,

    /// Peers this node can communicate with.
    #[serde(default)]
    pub peers: Vec<Node>,

    /// DNS configuration.
    #[serde(default)]
    pub dns_config: Option<DnsConfig>,

    /// DERP relay map.
    #[serde(default)]
    pub derp_map: Option<DerpMap>,

    /// User profiles referenced by peer nodes.
    #[serde(default)]
    pub user_profiles: Vec<User>,

    /// Domain name for MagicDNS.
    #[serde(default)]
    pub domain: Option<String>,

    /// Packet filter rules (compiled from ACLs).
    #[serde(default)]
    pub packet_filter: Vec<FilterRule>,

    /// Whether this is a full map or a delta update.
    #[serde(default)]
    pub is_delta: bool,

    /// Server-side collection URL (for debugging).
    #[serde(default)]
    pub collect_services: Option<bool>,
}

/// DNS configuration pushed to nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsConfig {
    /// Nameserver addresses.
    pub nameservers: Vec<String>,
    /// Search domains.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Per-domain resolvers (split DNS).
    #[serde(default)]
    pub routes: HashMap<String, Vec<String>>,
    /// Whether MagicDNS is enabled.
    #[serde(default)]
    pub magic_dns: bool,
}

/// DERP relay map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerpMap {
    /// Regions indexed by region ID.
    pub regions: HashMap<u16, DerpRegion>,
}

/// A DERP relay region.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerpRegion {
    pub region_id: u16,
    pub region_code: String,
    pub region_name: String,
    pub nodes: Vec<DerpNode>,
}

/// A single DERP relay node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerpNode {
    pub name: String,
    #[serde(rename = "regionID")]
    pub region_id: u16,
    pub host_name: String,
    #[serde(default)]
    pub ipv4: Option<String>,
    #[serde(default)]
    pub ipv6: Option<String>,
    #[serde(default)]
    pub stun_port: Option<u16>,
    #[serde(default)]
    pub derp_port: Option<u16>,
}

/// A compiled packet filter rule (ACL -> filter).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterRule {
    /// Source CIDRs.
    pub src_ips: Vec<String>,
    /// Destination ports.
    pub dst_ports: Vec<DstPortRange>,
}

/// A destination port range in a filter rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DstPortRange {
    /// Destination IP or CIDR.
    pub ip: String,
    pub ports: PortRange,
}

/// Port range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRange {
    pub first: u16,
    pub last: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_serializes() {
        let req = RegisterRequest {
            node_key: "abc123==".into(),
            machine_key: "def456==".into(),
            hostname: "myhost".into(),
            os: "linux".into(),
            auth_key: Some("tskey-auth-xxx".into()),
            tags: vec!["tag:server".into()],
            endpoints: vec!["1.2.3.4:41641".into()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: RegisterRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hostname, "myhost");
        assert_eq!(parsed.tags, vec!["tag:server"]);
    }

    #[test]
    fn map_response_serializes() {
        let resp = MapResponse {
            node: Some(Node {
                id: 1,
                stable_id: "stable-1".into(),
                name: "mynode".into(),
                key: "nodekey==".into(),
                machine_key: "machinekey==".into(),
                addresses: vec!["100.64.0.1".into()],
                allowed_ips: vec!["100.64.0.0/22".into()],
                endpoints: vec![],
                derp: Some("1".into()),
                hostname: "myhost".into(),
                os: "linux".into(),
                tags: vec![],
                online: true,
                last_seen: None,
                user: Some(1),
                authorized: true,
                created: None,
                key_expiry: None,
            }),
            peers: vec![],
            dns_config: Some(DnsConfig {
                nameservers: vec!["100.64.0.1".into()],
                domains: vec!["mesh.local".into()],
                routes: HashMap::new(),
                magic_dns: true,
            }),
            derp_map: None,
            user_profiles: vec![],
            domain: Some("mesh.local".into()),
            packet_filter: vec![],
            is_delta: false,
            collect_services: None,
        };
        let json = serde_json::to_string_pretty(&resp).unwrap();
        assert!(json.contains("mynode"));
        assert!(json.contains("100.64.0.1"));

        let parsed: MapResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.node.unwrap().name, "mynode");
    }

    #[test]
    fn filter_rule_serializes() {
        let rule = FilterRule {
            src_ips: vec!["100.64.0.0/22".into()],
            dst_ports: vec![DstPortRange {
                ip: "100.64.0.5/32".into(),
                ports: PortRange {
                    first: 80,
                    last: 443,
                },
            }],
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: FilterRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.dst_ports[0].ports.first, 80);
    }

    #[test]
    fn pre_auth_key_serializes() {
        let key = PreAuthKey {
            key: "tskey-auth-abc123".into(),
            tenant_id: "tenant-1".into(),
            user: "admin@example.com".into(),
            reusable: false,
            ephemeral: false,
            acl_tags: vec!["tag:server".into()],
            expiration: "2026-12-31T23:59:59Z".into(),
            used: false,
        };
        let json = serde_json::to_string(&key).unwrap();
        let parsed: PreAuthKey = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.acl_tags, vec!["tag:server"]);
    }
}
