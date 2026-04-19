//! Tailscale-compatible ACL policy data model.
//!
//! Represents the full ACL policy document with groups, host aliases,
//! tag owners, ACL rules, SSH rules, and auto-approvers.

use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::AclError;

/// A complete ACL policy document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AclPolicy {
    /// Named groups of users/identities.
    /// Keys must start with `group:`.
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,

    /// Named host aliases mapping to IP addresses or CIDRs.
    #[serde(default)]
    pub hosts: HashMap<String, String>,

    /// Tag ownership: which users can assign which tags.
    /// Keys must start with `tag:`.
    #[serde(default)]
    pub tag_owners: HashMap<String, Vec<String>>,

    /// The main access control rules.
    #[serde(default)]
    pub acls: Vec<AclRule>,

    /// SSH access rules.
    #[serde(default)]
    pub ssh: Vec<SshRule>,

    /// Auto-approvers for subnet routes and exit nodes.
    #[serde(default)]
    pub auto_approvers: Option<AutoApprovers>,

    /// Node attributes (conditional settings per node).
    #[serde(default)]
    pub node_attrs: Vec<NodeAttrRule>,

    /// Named IP sets (reusable network segments).
    #[serde(default)]
    pub ipsets: HashMap<String, Vec<String>>,

    /// Device posture conditions.
    #[serde(default)]
    pub postures: HashMap<String, Vec<String>>,

    /// Inline ACL tests (assertions that validate the policy).
    #[serde(default)]
    pub tests: Vec<AclTest>,

    /// Inline SSH tests.
    #[serde(default)]
    pub ssh_tests: Vec<SshTest>,

    /// Custom DERP relay map (opaque, forwarded to clients).
    #[serde(default)]
    pub derp_map: Option<serde_json::Value>,
}

/// A single ACL rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    /// "accept" or "deny" (only "accept" is currently used by Tailscale).
    pub action: AclAction,

    /// Source identities: users, groups, tags, CIDRs, or "*".
    pub src: Vec<String>,

    /// Destination targets in "host:port" format.
    /// Ports can be: "*", "80", "80-443", or a named port group.
    pub dst: Vec<String>,

    /// Optional IP protocol filter (tcp, udp, icmp, or protocol number).
    #[serde(default)]
    pub proto: Option<String>,
}

/// ACL action type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AclAction {
    Accept,
    Deny,
}

/// SSH rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshRule {
    pub action: SshAction,
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub users: Vec<String>,
    #[serde(default)]
    pub check_period: Option<String>,
    #[serde(default)]
    pub accept_env: Option<Vec<String>>,
}

/// SSH action type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SshAction {
    Accept,
    Check,
}

/// Auto-approvers for routes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoApprovers {
    /// CIDR -> list of users/groups that can auto-approve routes to it.
    #[serde(default)]
    pub routes: HashMap<String, Vec<String>>,

    /// Users/groups whose exit node advertisements are auto-approved.
    #[serde(default)]
    pub exit_node: Vec<String>,
}

/// Node attribute rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAttrRule {
    pub target: Vec<String>,
    #[serde(default)]
    pub attr: Vec<String>,
}

/// Inline ACL test assertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AclTest {
    pub src: String,
    #[serde(default)]
    pub proto: Option<String>,
    #[serde(default)]
    pub accept: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub src_posture_attrs: Option<HashMap<String, String>>,
}

/// Inline SSH test assertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTest {
    pub src: String,
    pub dst: Vec<String>,
    #[serde(default)]
    pub accept: Vec<String>,
    #[serde(default)]
    pub check: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub src_posture_attrs: Option<HashMap<String, String>>,
}

/// A parsed destination target (host:ports).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DstTarget {
    pub host: AclTarget,
    pub ports: PortSet,
}

/// Identifies a source or destination in an ACL rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AclTarget {
    /// Wildcard: matches everything.
    Any,
    /// A group reference: `group:name`.
    Group(String),
    /// A user identity: `user@domain.com`.
    User(String),
    /// A tag: `tag:name`.
    Tag(String),
    /// An IP address.
    Ip(IpAddr),
    /// A CIDR range.
    Cidr(IpNet),
    /// A host alias (defined in the `hosts` section).
    HostAlias(String),
    /// Autogroup like `autogroup:internet`.
    Autogroup(String),
}

/// A set of ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortSet {
    /// All ports.
    Any,
    /// Specific ports and ranges.
    Ports(Vec<PortRange>),
}

/// A port or port range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

impl PortRange {
    pub fn single(port: u16) -> Self {
        Self {
            start: port,
            end: port,
        }
    }

    pub fn range(start: u16, end: u16) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, port: u16) -> bool {
        port >= self.start && port <= self.end
    }
}

impl PortSet {
    pub fn contains(&self, port: u16) -> bool {
        match self {
            PortSet::Any => true,
            PortSet::Ports(ranges) => ranges.iter().any(|r| r.contains(port)),
        }
    }
}

impl AclPolicy {
    /// Parse an ACL policy from a HuJSON string.
    pub fn from_hujson(input: &str) -> Result<Self, AclError> {
        let json = crate::hujson::to_json(input)?;
        let policy: AclPolicy =
            serde_json::from_str(&json).map_err(|e| AclError::InvalidPolicy(e.to_string()))?;
        policy.validate()?;
        Ok(policy)
    }

    /// Parse an ACL policy from a JSON string (no HuJSON extensions).
    pub fn from_json(input: &str) -> Result<Self, AclError> {
        let policy: AclPolicy =
            serde_json::from_str(input).map_err(|e| AclError::InvalidPolicy(e.to_string()))?;
        policy.validate()?;
        Ok(policy)
    }

    /// Validate the policy for internal consistency.
    pub fn validate(&self) -> Result<(), AclError> {
        // Check group names start with group:
        for key in self.groups.keys() {
            if !key.starts_with("group:") {
                return Err(AclError::InvalidPolicy(format!(
                    "group key must start with 'group:': {key}"
                )));
            }
        }

        // Check tag_owners keys start with tag:
        for key in self.tag_owners.keys() {
            if !key.starts_with("tag:") {
                return Err(AclError::InvalidPolicy(format!(
                    "tagOwners key must start with 'tag:': {key}"
                )));
            }
        }

        // Check for circular group references
        for group_name in self.groups.keys() {
            let mut visited = std::collections::HashSet::new();
            self.check_group_cycle(group_name, &mut visited)?;
        }

        Ok(())
    }

    fn check_group_cycle(
        &self,
        name: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<(), AclError> {
        if !visited.insert(name.to_string()) {
            return Err(AclError::CircularGroup(name.to_string()));
        }
        if let Some(members) = self.groups.get(name) {
            for member in members {
                if member.starts_with("group:") {
                    self.check_group_cycle(member, visited)?;
                }
            }
        }
        visited.remove(name);
        Ok(())
    }

    /// Resolve a group name to all its members (recursively expanding nested groups).
    pub fn resolve_group(&self, name: &str) -> Result<Vec<String>, AclError> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.resolve_group_inner(name, &mut result, &mut visited)?;
        Ok(result)
    }

    fn resolve_group_inner(
        &self,
        name: &str,
        result: &mut Vec<String>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<(), AclError> {
        if !visited.insert(name.to_string()) {
            return Err(AclError::CircularGroup(name.to_string()));
        }
        let members = self
            .groups
            .get(name)
            .ok_or_else(|| AclError::UnknownGroup(name.to_string()))?;
        for member in members {
            if member.starts_with("group:") {
                self.resolve_group_inner(member, result, visited)?;
            } else {
                result.push(member.clone());
            }
        }
        visited.remove(name);
        Ok(())
    }

    /// Resolve a host alias to its IP/CIDR.
    pub fn resolve_host(&self, name: &str) -> Result<IpNet, AclError> {
        let addr = self
            .hosts
            .get(name)
            .ok_or_else(|| AclError::UnknownHost(name.to_string()))?;
        parse_ip_or_cidr(addr)
    }
}

/// Parse a source string into an AclTarget.
pub fn parse_target(s: &str) -> AclTarget {
    if s == "*" {
        return AclTarget::Any;
    }
    if let Some(name) = s.strip_prefix("group:") {
        return AclTarget::Group(format!("group:{name}"));
    }
    if let Some(name) = s.strip_prefix("tag:") {
        return AclTarget::Tag(format!("tag:{name}"));
    }
    if let Some(name) = s.strip_prefix("autogroup:") {
        return AclTarget::Autogroup(name.to_string());
    }
    if let Ok(net) = IpNet::from_str(s) {
        return AclTarget::Cidr(net);
    }
    if let Ok(ip) = IpAddr::from_str(s) {
        return AclTarget::Ip(ip);
    }
    if s.contains('@') {
        return AclTarget::User(s.to_string());
    }
    AclTarget::HostAlias(s.to_string())
}

/// Parse a destination string like "host:port" or "host:port1,port2" or "*:*".
pub fn parse_dst(s: &str) -> Result<DstTarget, AclError> {
    if s == "*:*" {
        return Ok(DstTarget {
            host: AclTarget::Any,
            ports: PortSet::Any,
        });
    }

    // Find the last colon that separates host from port
    // Handle IPv6: [::1]:80
    let (host_part, port_part) = if s.starts_with('[') {
        // IPv6 literal
        let bracket_end = s
            .find(']')
            .ok_or_else(|| AclError::InvalidPolicy(format!("unterminated IPv6 bracket: {s}")))?;
        let host = &s[..=bracket_end];
        let rest = &s[bracket_end + 1..];
        if let Some(port) = rest.strip_prefix(':') {
            (host, port)
        } else {
            (host, "*")
        }
    } else if let Some(colon_pos) = s.rfind(':') {
        (&s[..colon_pos], &s[colon_pos + 1..])
    } else {
        (s, "*")
    };

    let host = parse_target(host_part);
    let ports = parse_ports(port_part)?;

    Ok(DstTarget { host, ports })
}

/// Parse a port specification: "*", "80", "80-443", "80,443,8080-8090".
pub fn parse_ports(s: &str) -> Result<PortSet, AclError> {
    if s == "*" {
        return Ok(PortSet::Any);
    }

    let mut ranges = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let start: u16 = start
                .trim()
                .parse()
                .map_err(|_| AclError::InvalidPortRange(part.to_string()))?;
            let end: u16 = end
                .trim()
                .parse()
                .map_err(|_| AclError::InvalidPortRange(part.to_string()))?;
            ranges.push(PortRange::range(start, end));
        } else {
            let port: u16 = part
                .parse()
                .map_err(|_| AclError::InvalidPortRange(part.to_string()))?;
            ranges.push(PortRange::single(port));
        }
    }

    Ok(PortSet::Ports(ranges))
}

/// Parse an IP address or CIDR string.
fn parse_ip_or_cidr(s: &str) -> Result<IpNet, AclError> {
    if let Ok(net) = IpNet::from_str(s) {
        return Ok(net);
    }
    if let Ok(ip) = IpAddr::from_str(s) {
        let prefix = if ip.is_ipv4() { 32 } else { 128 };
        return IpNet::new(ip, prefix).map_err(|e| AclError::InvalidCidr(e.to_string()));
    }
    Err(AclError::InvalidCidr(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_policy() {
        let input = r#"{
            "groups": {
                "group:admin": ["admin@example.com"]
            },
            "acls": [
                {"action": "accept", "src": ["group:admin"], "dst": ["*:*"]}
            ]
        }"#;
        let policy = AclPolicy::from_json(input).unwrap();
        assert_eq!(policy.groups.len(), 1);
        assert_eq!(policy.acls.len(), 1);
        assert_eq!(policy.acls[0].action, AclAction::Accept);
    }

    #[test]
    fn parse_targets() {
        assert_eq!(parse_target("*"), AclTarget::Any);
        assert!(matches!(
            parse_target("group:admin"),
            AclTarget::Group(ref s) if s == "group:admin"
        ));
        assert!(matches!(
            parse_target("tag:server"),
            AclTarget::Tag(ref s) if s == "tag:server"
        ));
        assert!(matches!(
            parse_target("user@example.com"),
            AclTarget::User(ref s) if s == "user@example.com"
        ));
        assert!(matches!(parse_target("10.0.0.0/8"), AclTarget::Cidr(_)));
        assert!(matches!(parse_target("10.0.0.1"), AclTarget::Ip(_)));
    }

    #[test]
    fn parse_dst_targets() {
        let dst = parse_dst("*:*").unwrap();
        assert_eq!(dst.host, AclTarget::Any);
        assert_eq!(dst.ports, PortSet::Any);

        let dst = parse_dst("tag:server:80").unwrap();
        assert!(matches!(dst.host, AclTarget::Tag(_)));
        assert!(dst.ports.contains(80));
        assert!(!dst.ports.contains(81));

        let dst = parse_dst("10.0.0.1:80-443").unwrap();
        assert!(dst.ports.contains(80));
        assert!(dst.ports.contains(443));
        assert!(dst.ports.contains(200));
        assert!(!dst.ports.contains(79));
    }

    #[test]
    fn resolve_nested_groups() {
        let input = r#"{
            "groups": {
                "group:team": ["alice@ex.com", "bob@ex.com"],
                "group:all": ["group:team", "carol@ex.com"]
            },
            "acls": []
        }"#;
        let policy = AclPolicy::from_json(input).unwrap();
        let members = policy.resolve_group("group:all").unwrap();
        assert_eq!(members.len(), 3);
        assert!(members.contains(&"alice@ex.com".to_string()));
        assert!(members.contains(&"carol@ex.com".to_string()));
    }

    #[test]
    fn detect_circular_groups() {
        let input = r#"{
            "groups": {
                "group:a": ["group:b"],
                "group:b": ["group:a"]
            },
            "acls": []
        }"#;
        assert!(AclPolicy::from_json(input).is_err());
    }

    #[test]
    fn invalid_group_key() {
        let input = r#"{
            "groups": {
                "admin": ["user@ex.com"]
            },
            "acls": []
        }"#;
        assert!(AclPolicy::from_json(input).is_err());
    }

    #[test]
    fn hujson_policy() {
        let input = r#"{
            // Admin group
            "groups": {
                "group:admin": [
                    "admin@example.com",
                ],
            },
            /* Network rules */
            "acls": [
                {
                    "action": "accept",
                    "src": ["group:admin"],
                    "dst": ["*:*"],
                },
            ],
        }"#;
        let policy = AclPolicy::from_hujson(input).unwrap();
        assert_eq!(policy.acls.len(), 1);
    }
}
