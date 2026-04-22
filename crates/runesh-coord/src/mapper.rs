//! Map builder: constructs per-node MapResponses from the full node set and ACLs.
//!
//! Each node gets a tailored view of the mesh: only the peers it's allowed
//! to communicate with (based on ACL evaluation), plus DNS and DERP config.

use std::collections::HashMap;

use runesh_acl::{AclPolicy, EvalContext, PortSet, parse_dst};

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

            if peer.addresses.is_empty() {
                continue;
            }

            // Check if self_node can reach this peer on any port
            let can_reach = self.can_communicate(self_node, peer);

            if can_reach {
                peers.push(peer.clone());
                if let Some(uid) = peer.user
                    && let Some(user) = self.users.get(&uid)
                    && !referenced_users.iter().any(|u: &User| u.id == uid)
                {
                    referenced_users.push(user.clone());
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
    ///
    /// Instead of probing a fixed list of ports, this walks the ACL's actual
    /// destination port specifications, unions them, and checks each distinct
    /// range against the evaluator. This correctly surfaces user-declared
    /// ranges like `8000-9000` or `1-1024` in the emitted filter.
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
        let dst_user = to
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
            dst_user,
            proto: None,
        };

        // Fast path: if rule evaluation at port 0 is allowed via a wildcard
        // port spec, treat as all ports allowed.
        let wildcard = EvalContext {
            dst_port: 0,
            ..base_ctx.clone()
        };
        if self.acl.evaluate(&wildcard).allowed {
            // Confirm wildcard by checking a sentinel high port.
            let sentinel = EvalContext {
                dst_port: 65535,
                ..base_ctx.clone()
            };
            if self.acl.evaluate(&sentinel).allowed {
                return vec![(0, 65535)];
            }
        }

        // Collect candidate port ranges directly from the ACL rules.
        let mut candidates: Vec<(u16, u16)> = Vec::new();
        for rule in &self.acl.acls {
            for d in &rule.dst {
                let parsed = match parse_dst(d) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                match parsed.ports {
                    PortSet::Any => candidates.push((0, 65535)),
                    PortSet::Ports(ranges) => {
                        for r in ranges {
                            candidates.push((r.start, r.end));
                        }
                    }
                }
            }
        }

        // Sort, dedup, and merge overlapping/adjacent ranges.
        candidates.sort();
        candidates.dedup();
        let candidates = merge_ranges(candidates);

        // For each candidate range, probe both endpoints. If both match, keep
        // the range. If only some ports match, split into the matching
        // sub-span. For simple single-port rules (start==end) this is exact.
        let mut allowed: Vec<(u16, u16)> = Vec::new();
        for (start, end) in candidates {
            let start_ok = {
                let ctx = EvalContext {
                    dst_port: start,
                    ..base_ctx.clone()
                };
                self.acl.evaluate(&ctx).allowed
            };
            let end_ok = if start == end {
                start_ok
            } else {
                let ctx = EvalContext {
                    dst_port: end,
                    ..base_ctx.clone()
                };
                self.acl.evaluate(&ctx).allowed
            };

            if start_ok && end_ok {
                allowed.push((start, end));
            } else if start_ok {
                allowed.push((start, start));
            } else if end_ok {
                allowed.push((end, end));
            }
        }

        allowed.sort();
        allowed.dedup();
        merge_ranges(allowed)
    }

    /// Build packet filter rules for a node based on ACL evaluation.
    fn build_packet_filter(&self, node: &Node) -> Vec<FilterRule> {
        let mut rules = Vec::new();

        let dst_ip = match node.addresses.first() {
            Some(ip) => ip.clone(),
            None => return rules,
        };

        for peer in self.nodes.values() {
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

/// Merge sorted port ranges, collapsing overlapping or adjacent spans.
fn merge_ranges(mut ranges: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort();
    let mut merged: Vec<(u16, u16)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        if let Some(last) = merged.last_mut()
            && s <= last.1.saturating_add(1)
        {
            if e > last.1 {
                last.1 = e;
            }
            continue;
        }
        merged.push((s, e));
    }
    merged
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
