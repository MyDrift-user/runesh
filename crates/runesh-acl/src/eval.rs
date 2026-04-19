//! ACL evaluation engine.
//!
//! Given a source identity, destination peer, and port, evaluates the
//! ACL rules and returns whether the connection is allowed.

use std::net::IpAddr;

use crate::model::{AclAction, AclPolicy, AclTarget, DstTarget, parse_dst, parse_target};

/// Context for evaluating an ACL rule against a specific connection.
#[derive(Debug, Clone)]
pub struct EvalContext {
    /// Source identity (user email, e.g., "alice@example.com").
    pub src_user: Option<String>,
    /// Groups the source user belongs to (e.g., ["group:admin"]).
    pub src_groups: Vec<String>,
    /// Tags assigned to the source device (e.g., ["tag:server"]).
    pub src_tags: Vec<String>,
    /// Source device mesh IP.
    pub src_ip: IpAddr,

    /// Destination device mesh IP.
    pub dst_ip: IpAddr,
    /// Tags assigned to the destination device.
    pub dst_tags: Vec<String>,
    /// Destination port.
    pub dst_port: u16,
}

/// Result of evaluating an ACL policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclEvalResult {
    /// Whether the connection is allowed.
    pub allowed: bool,
    /// Index of the matching rule (if any).
    pub matching_rule: Option<usize>,
    /// The action of the matching rule.
    pub action: Option<AclAction>,
}

impl AclEvalResult {
    fn default_deny() -> Self {
        Self {
            allowed: false,
            matching_rule: None,
            action: None,
        }
    }
}

impl AclPolicy {
    /// Evaluate whether a connection described by `ctx` is allowed by this policy.
    ///
    /// Rules are evaluated in order. The first matching rule wins.
    /// If no rule matches, the connection is denied (default deny).
    pub fn evaluate(&self, ctx: &EvalContext) -> AclEvalResult {
        for (i, rule) in self.acls.iter().enumerate() {
            let src_matches = rule.src.iter().any(|s| {
                let target = parse_target(s);
                self.matches_source(&target, ctx)
            });

            if !src_matches {
                continue;
            }

            let dst_matches = rule.dst.iter().any(|d| match parse_dst(d) {
                Ok(dst_target) => self.matches_destination(&dst_target, ctx),
                Err(_) => false,
            });

            if !dst_matches {
                continue;
            }

            return AclEvalResult {
                allowed: rule.action == AclAction::Accept,
                matching_rule: Some(i),
                action: Some(rule.action.clone()),
            };
        }

        AclEvalResult::default_deny()
    }

    /// Check if a source target matches the evaluation context.
    fn matches_source(&self, target: &AclTarget, ctx: &EvalContext) -> bool {
        match target {
            AclTarget::Any => true,
            AclTarget::User(user) => ctx.src_user.as_deref() == Some(user.as_str()),
            AclTarget::Group(group) => {
                // Check if user is a direct member or if any of their groups match
                if ctx.src_groups.contains(group) {
                    return true;
                }
                // Try resolving the group and checking membership
                if let Some(user) = &ctx.src_user {
                    if let Ok(members) = self.resolve_group(group) {
                        return members.contains(user);
                    }
                }
                false
            }
            AclTarget::Tag(tag) => ctx.src_tags.iter().any(|t| t == tag),
            AclTarget::Ip(ip) => ctx.src_ip == *ip,
            AclTarget::Cidr(net) => net.contains(&ctx.src_ip),
            AclTarget::HostAlias(name) => {
                if let Ok(net) = self.resolve_host(name.as_str()) {
                    net.contains(&ctx.src_ip)
                } else {
                    false
                }
            }
            AclTarget::Autogroup(name) => match name.as_str() {
                "member" => ctx.src_user.is_some(),
                "tagged" => !ctx.src_tags.is_empty(),
                _ => false,
            },
        }
    }

    /// Check if a destination target matches the evaluation context.
    fn matches_destination(&self, dst: &DstTarget, ctx: &EvalContext) -> bool {
        let host_matches = match &dst.host {
            AclTarget::Any => true,
            AclTarget::Tag(tag) => ctx.dst_tags.iter().any(|t| t == tag),
            AclTarget::Ip(ip) => ctx.dst_ip == *ip,
            AclTarget::Cidr(net) => net.contains(&ctx.dst_ip),
            AclTarget::HostAlias(name) => {
                if let Ok(net) = self.resolve_host(name.as_str()) {
                    net.contains(&ctx.dst_ip)
                } else {
                    false
                }
            }
            AclTarget::User(user) => ctx.src_user.as_deref() == Some(user.as_str()),
            AclTarget::Group(_) => {
                // Groups in dst refer to devices owned by group members.
                // Requires device-to-user mapping from mesh context.
                false
            }
            AclTarget::Autogroup(name) => match name.as_str() {
                "internet" => true,
                "self" => ctx.src_ip == ctx.dst_ip,
                _ => false,
            },
        };

        host_matches && dst.ports.contains(ctx.dst_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_policy() -> AclPolicy {
        AclPolicy::from_json(
            r#"{
            "groups": {
                "group:admin": ["admin@example.com"],
                "group:dev": ["dev@example.com", "admin@example.com"]
            },
            "hosts": {
                "server1": "100.64.0.10"
            },
            "acls": [
                {
                    "action": "accept",
                    "src": ["group:admin"],
                    "dst": ["*:*"]
                },
                {
                    "action": "accept",
                    "src": ["group:dev"],
                    "dst": ["tag:webserver:80,443"]
                },
                {
                    "action": "accept",
                    "src": ["*"],
                    "dst": ["*:53"]
                }
            ]
        }"#,
        )
        .unwrap()
    }

    fn ctx(src_user: &str, src_groups: &[&str], dst_ip: &str, dst_port: u16) -> EvalContext {
        EvalContext {
            src_user: Some(src_user.to_string()),
            src_groups: src_groups.iter().map(|s| s.to_string()).collect(),
            src_tags: vec![],
            src_ip: "100.64.0.1".parse().unwrap(),
            dst_ip: dst_ip.parse().unwrap(),
            dst_tags: vec![],
            dst_port,
        }
    }

    #[test]
    fn admin_can_access_everything() {
        let policy = test_policy();
        let result = policy.evaluate(&ctx("admin@example.com", &[], "100.64.0.10", 22));
        assert!(result.allowed);
        assert_eq!(result.matching_rule, Some(0));
    }

    #[test]
    fn dev_can_access_tagged_webserver() {
        let policy = test_policy();
        let mut c = ctx("dev@example.com", &[], "100.64.0.10", 80);
        c.dst_tags = vec!["tag:webserver".to_string()];
        let result = policy.evaluate(&c);
        assert!(result.allowed);
        assert_eq!(result.matching_rule, Some(1));
    }

    #[test]
    fn dev_cannot_access_ssh() {
        let policy = test_policy();
        let mut c = ctx("dev@example.com", &[], "100.64.0.10", 22);
        c.dst_tags = vec!["tag:webserver".to_string()];
        let result = policy.evaluate(&c);
        // Should match rule 2 (everyone on port 53) or deny
        // Port 22 != 53, and dev can only reach webserver:80,443
        assert!(!result.allowed || result.matching_rule == Some(2));
    }

    #[test]
    fn everyone_can_access_dns() {
        let policy = test_policy();
        let result = policy.evaluate(&ctx("random@example.com", &[], "100.64.0.50", 53));
        assert!(result.allowed);
        assert_eq!(result.matching_rule, Some(2));
    }

    #[test]
    fn default_deny() {
        let policy = test_policy();
        let result = policy.evaluate(&ctx("random@example.com", &[], "100.64.0.50", 22));
        assert!(!result.allowed);
        assert_eq!(result.matching_rule, None);
    }

    #[test]
    fn empty_policy_denies_all() {
        let policy = AclPolicy::from_json(r#"{"acls": []}"#).unwrap();
        let result = policy.evaluate(&ctx("user@ex.com", &[], "10.0.0.1", 80));
        assert!(!result.allowed);
    }

    #[test]
    fn cidr_source_matching() {
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["100.64.0.0/24"], "dst": ["*:*"]}
            ]
        }"#,
        )
        .unwrap();

        let mut c = ctx("user@ex.com", &[], "10.0.0.1", 80);
        c.src_ip = "100.64.0.5".parse().unwrap();
        assert!(policy.evaluate(&c).allowed);

        c.src_ip = "100.64.1.5".parse().unwrap();
        assert!(!policy.evaluate(&c).allowed);
    }
}
