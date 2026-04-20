//! Map builder: constructs per-node MapResponses from the full node set and ACLs.
//!
//! Each node gets a tailored view of the mesh: only the peers it's allowed
//! to communicate with (based on ACL evaluation), plus DNS and DERP config.

use std::collections::HashMap;

use runesh_acl::{AclPolicy, EvalContext};

use crate::types::{
    DerpMap, DnsConfig, DstPortRange, FilterRule, MapResponse, Node, PortRange, User,
};

/// Builds MapResponses for nodes.
pub struct MapBuilder {
    /// All nodes in the mesh.
    nodes: HashMap<u64, Node>,
    /// All users.
    users: HashMap<u64, User>,
    /// Current ACL policy.
    acl: AclPolicy,
    /// DERP relay map.
    derp_map: Option<DerpMap>,
    /// DNS configuration.
    dns_config: Option<DnsConfig>,
    /// Mesh domain.
    domain: Option<String>,
}

impl MapBuilder {
    pub fn new(acl: AclPolicy) -> Self {
        Self {
            nodes: HashMap::new(),
            users: HashMap::new(),
            acl,
            derp_map: None,
            dns_config: None,
            domain: None,
        }
    }

    pub fn set_derp_map(&mut self, map: DerpMap) {
        self.derp_map = Some(map);
    }

    pub fn set_dns_config(&mut self, config: DnsConfig) {
        self.dns_config = Some(config);
    }

    pub fn set_domain(&mut self, domain: String) {
        self.domain = Some(domain);
    }

    pub fn set_acl(&mut self, acl: AclPolicy) {
        self.acl = acl;
    }

    pub fn upsert_node(&mut self, node: Node) {
        self.nodes.insert(node.id, node);
    }

    pub fn remove_node(&mut self, id: u64) {
        self.nodes.remove(&id);
    }

    pub fn upsert_user(&mut self, user: User) {
        self.users.insert(user.id, user);
    }

    /// Build a MapResponse for a specific node.
    ///
    /// The response contains only peers this node is allowed to reach
    /// (filtered by ACL evaluation) and the compiled packet filter.
    pub fn build_map(&self, node_id: u64) -> Option<MapResponse> {
        let self_node = self.nodes.get(&node_id)?;
        let _self_ip = self_node.addresses.first()?;

        // Find peers this node can communicate with
        let mut peers = Vec::new();
        let mut referenced_users = Vec::new();

        for (peer_id, peer) in &self.nodes {
            if *peer_id == node_id {
                continue;
            }
            if !peer.authorized {
                continue;
            }

            if peer.addresses.first().is_none() {
                continue;
            }

            // Check if self_node can reach this peer on any port
            let can_reach = self.can_communicate(self_node, peer);

            if can_reach {
                peers.push(peer.clone());
                if let Some(uid) = peer.user {
                    if let Some(user) = self.users.get(&uid) {
                        if !referenced_users.iter().any(|u: &User| u.id == uid) {
                            referenced_users.push(user.clone());
                        }
                    }
                }
            }
        }

        // Build packet filter from ACL rules
        let packet_filter = self.build_packet_filter(self_node);

        Some(MapResponse {
            node: Some(self_node.clone()),
            peers,
            dns_config: self.dns_config.clone(),
            derp_map: self.derp_map.clone(),
            user_profiles: referenced_users,
            domain: self.domain.clone(),
            packet_filter,
            is_delta: false,
            collect_services: None,
        })
    }

    /// Check if node A can communicate with node B on any port.
    /// Scans all ACL rules to find if any allows traffic between these nodes.
    fn can_communicate(&self, from: &Node, to: &Node) -> bool {
        !self.allowed_ports(from, to).is_empty()
    }

    /// Determine which ports node `from` can reach on node `to`.
    /// Returns a list of (start, end) port ranges.
    fn allowed_ports(&self, from: &Node, to: &Node) -> Vec<(u16, u16)> {
        let src_ip = match from.addresses.first().and_then(|ip| ip.parse().ok()) {
            Some(ip) => ip,
            None => return vec![],
        };
        let dst_ip = match to.addresses.first().and_then(|ip| ip.parse().ok()) {
            Some(ip) => ip,
            None => return vec![],
        };

        let src_user = from
            .user
            .and_then(|uid| self.users.get(&uid).map(|u| u.login_name.clone()));

        let base_ctx = EvalContext {
            src_user,
            src_groups: vec![],
            src_tags: from.tags.clone(),
            src_ip,
            dst_ip,
            dst_tags: to.tags.clone(),
            dst_port: 0,
        };

        // Check if wildcard port (0) is allowed, meaning all ports
        let wildcard = EvalContext {
            dst_port: 0,
            ..base_ctx.clone()
        };
        if self.acl.evaluate(&wildcard).allowed {
            return vec![(0, 65535)];
        }

        // Scan representative ports to find which ranges are allowed
        let mut allowed = Vec::new();
        let probe_ports = [
            22, 25, 53, 80, 443, 465, 587, 993, 995, 1433, 1521, 3306, 3389, 5432, 5900, 6379,
            8080, 8443, 9090, 9100, 9200, 27017,
        ];

        for &port in &probe_ports {
            let ctx = EvalContext {
                dst_port: port,
                ..base_ctx.clone()
            };
            if self.acl.evaluate(&ctx).allowed {
                allowed.push((port, port));
            }
        }

        // Also check common ranges
        for (start, end) in [(1024, 1024), (8000, 8000), (49152, 49152)] {
            let ctx = EvalContext {
                dst_port: start,
                ..base_ctx.clone()
            };
            if self.acl.evaluate(&ctx).allowed {
                allowed.push((start, end));
            }
        }

        // Merge adjacent/overlapping ranges
        allowed.sort();
        allowed.dedup();
        allowed
    }

    /// Build packet filter rules for a node based on ACL evaluation.
    fn build_packet_filter(&self, node: &Node) -> Vec<FilterRule> {
        let mut rules = Vec::new();

        let dst_ip = match node.addresses.first() {
            Some(ip) => ip.clone(),
            None => return rules,
        };

        for (_, peer) in &self.nodes {
            if peer.id == node.id || !peer.authorized {
                continue;
            }
            let src_ip = match peer.addresses.first() {
                Some(ip) => ip.clone(),
                None => continue,
            };

            let ports = self.allowed_ports(peer, node);
            if ports.is_empty() {
                continue;
            }

            let dst_ports: Vec<DstPortRange> = ports
                .iter()
                .map(|(start, end)| DstPortRange {
                    ip: format!("{dst_ip}/32"),
                    ports: PortRange {
                        first: *start,
                        last: *end,
                    },
                })
                .collect();

            rules.push(FilterRule {
                src_ips: vec![format!("{src_ip}/32")],
                dst_ports,
            });
        }

        rules
    }

    /// Get all node IDs.
    pub fn node_ids(&self) -> Vec<u64> {
        self.nodes.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: u64, name: &str, ip: &str, tags: &[&str]) -> Node {
        Node {
            id,
            stable_id: format!("stable-{id}"),
            name: name.into(),
            key: format!("key-{id}"),
            machine_key: format!("mkey-{id}"),
            addresses: vec![ip.into()],
            allowed_ips: vec![],
            endpoints: vec![],
            derp: None,
            hostname: name.into(),
            os: "linux".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            online: true,
            last_seen: None,
            user: Some(1),
            authorized: true,
            created: None,
            key_expiry: None,
        }
    }

    #[test]
    fn build_map_for_node() {
        let acl = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();

        let mut builder = MapBuilder::new(acl);
        builder.upsert_user(User {
            id: 1,
            login_name: "admin@example.com".into(),
            display_name: "Admin".into(),
            roles: vec![],
        });
        builder.upsert_node(make_node(1, "node-a", "100.64.0.1", &[]));
        builder.upsert_node(make_node(2, "node-b", "100.64.0.2", &[]));
        builder.upsert_node(make_node(3, "node-c", "100.64.0.3", &[]));

        let map = builder.build_map(1).unwrap();
        assert_eq!(map.peers.len(), 2);
        assert!(map.node.is_some());
        assert_eq!(map.node.unwrap().id, 1);
    }

    #[test]
    fn acl_filters_peers() {
        let acl = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["tag:admin"], "dst": ["*:*"]}
            ]
        }"#,
        )
        .unwrap();

        let mut builder = MapBuilder::new(acl);
        builder.upsert_node(make_node(1, "admin", "100.64.0.1", &["tag:admin"]));
        builder.upsert_node(make_node(2, "server", "100.64.0.2", &[]));
        builder.upsert_node(make_node(3, "other", "100.64.0.3", &[]));

        // Admin node can see server and other (it has tag:admin, ACL allows tag:admin -> *)
        let map = builder.build_map(1).unwrap();
        assert_eq!(map.peers.len(), 2);

        // Other node cannot see anything (no tag:admin, no matching rule)
        let map = builder.build_map(3).unwrap();
        assert_eq!(map.peers.len(), 0);
    }

    #[test]
    fn unauthorized_nodes_excluded() {
        let acl = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();

        let mut builder = MapBuilder::new(acl);
        builder.upsert_node(make_node(1, "authorized", "100.64.0.1", &[]));
        let mut unauth = make_node(2, "pending", "100.64.0.2", &[]);
        unauth.authorized = false;
        builder.upsert_node(unauth);

        let map = builder.build_map(1).unwrap();
        assert_eq!(map.peers.len(), 0); // unauthorized peer excluded
    }

    #[test]
    fn nonexistent_node_returns_none() {
        let acl = AclPolicy::from_json(r#"{"acls": []}"#).unwrap();
        let builder = MapBuilder::new(acl);
        assert!(builder.build_map(999).is_none());
    }
}
