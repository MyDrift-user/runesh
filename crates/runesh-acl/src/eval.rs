//! ACL evaluation engine.
//!
//! Given a source identity, destination peer, and port, evaluates the
//! ACL rules and returns whether the connection is allowed.

use std::net::IpAddr;
use std::sync::Arc;

use crate::AclError;
use crate::model::{
    AclAction, AclPolicy, AclTarget, DstTarget, SshAction, parse_dst, parse_target,
};

/// Resolves a group name to the users (device owners) that belong to it.
///
/// Implementations should expand nested groups if desired.
pub trait GroupResolver: Send + Sync {
    /// Return the list of user identities that are members of `group`.
    fn members(&self, group: &str) -> Vec<String>;
}

/// IP protocol for proto-aware rule matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
    Icmp,
    Sctp,
    /// Numeric IP protocol; use for anything the enum does not name.
    Number(u8),
}

impl Proto {
    /// Parse a rule's `proto` field. Accepts "tcp", "udp", "icmp", "sctp",
    /// a decimal protocol number, or the wildcards "*" / "any" which map
    /// to `None` (match any proto).
    pub fn parse_rule_field(raw: &str) -> Option<Proto> {
        let s = raw.trim().to_ascii_lowercase();
        match s.as_str() {
            "" | "*" | "any" => None,
            "tcp" => Some(Proto::Tcp),
            "udp" => Some(Proto::Udp),
            "icmp" => Some(Proto::Icmp),
            "sctp" => Some(Proto::Sctp),
            other => other.parse::<u8>().ok().map(Proto::Number),
        }
    }

    /// True when the rule's declared proto is compatible with the context's
    /// probed proto. A rule that does not set a proto matches any ctx proto.
    /// A ctx that does not set a proto matches any rule proto (so proto
    /// filtering is opt-in at both the policy side and the call site).
    pub fn rule_matches(rule: Option<Proto>, ctx: Option<Proto>) -> bool {
        match (rule, ctx) {
            (None, _) | (_, None) => true,
            (Some(a), Some(b)) => a == b,
        }
    }
}

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
    /// Destination device owner (for resolving `group:` targets in dst).
    /// When `None`, `Group` targets in dst cannot be matched.
    pub dst_user: Option<String>,
    /// Transport protocol of the probed connection. `None` means the caller
    /// does not care and every rule's `proto` is considered matching.
    #[allow(clippy::struct_field_names)]
    pub proto: Option<Proto>,
}

impl Default for EvalContext {
    fn default() -> Self {
        Self {
            src_user: None,
            src_groups: vec![],
            src_tags: vec![],
            src_ip: "0.0.0.0".parse().unwrap(),
            dst_ip: "0.0.0.0".parse().unwrap(),
            dst_tags: vec![],
            dst_port: 0,
            dst_user: None,
            proto: None,
        }
    }
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

/// An ACL evaluator bound to a policy, optionally enriched with a group
/// resolver for resolving device-owner groups referenced in destination
/// targets.
pub struct AclEvaluator<'a> {
    policy: &'a AclPolicy,
    group_resolver: Option<Arc<dyn GroupResolver>>,
}

impl<'a> AclEvaluator<'a> {
    pub fn new(policy: &'a AclPolicy) -> Self {
        Self {
            policy,
            group_resolver: None,
        }
    }

    /// Attach a group resolver used to expand `group:` targets that appear
    /// in destination position.
    pub fn with_group_resolver(mut self, resolver: Arc<dyn GroupResolver>) -> Self {
        self.group_resolver = Some(resolver);
        self
    }

    /// Evaluate the policy, returning an explicit error when a feature is
    /// requested that cannot be satisfied (e.g. a `group:` target in dst
    /// position with no resolver configured).
    pub fn try_evaluate(&self, ctx: &EvalContext) -> Result<AclEvalResult, AclError> {
        for (i, rule) in self.policy.acls.iter().enumerate() {
            // Proto filter: if the rule declares a proto and the context
            // declares one, they must agree. Both sides default to "any"
            // when unset so existing callers keep working.
            let rule_proto = rule.proto.as_deref().and_then(Proto::parse_rule_field);
            if !Proto::rule_matches(rule_proto, ctx.proto) {
                continue;
            }

            let src_matches = rule.src.iter().any(|s| {
                let target = parse_target(s);
                self.policy.matches_source(&target, ctx)
            });

            if !src_matches {
                continue;
            }

            let mut dst_matches = false;
            for d in &rule.dst {
                let dst_target = match parse_dst(d) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if self.matches_destination(&dst_target, ctx)? {
                    dst_matches = true;
                    break;
                }
            }

            if !dst_matches {
                continue;
            }

            return Ok(AclEvalResult {
                allowed: rule.action == AclAction::Accept,
                matching_rule: Some(i),
                action: Some(rule.action.clone()),
            });
        }

        Ok(AclEvalResult::default_deny())
    }

    fn matches_destination(&self, dst: &DstTarget, ctx: &EvalContext) -> Result<bool, AclError> {
        let host_matches = match &dst.host {
            AclTarget::Any => true,
            AclTarget::Tag(tag) => ctx.dst_tags.iter().any(|t| t == tag),
            AclTarget::Ip(ip) => ctx.dst_ip == *ip,
            AclTarget::Cidr(net) => net.contains(&ctx.dst_ip),
            AclTarget::HostAlias(name) => self
                .policy
                .resolve_host(name.as_str())
                .map(|net| net.contains(&ctx.dst_ip))
                .unwrap_or(false),
            AclTarget::User(user) => ctx.dst_user.as_deref() == Some(user.as_str()),
            AclTarget::Group(group) => {
                // Prefer the mesh-context resolver; fall back to the in-policy
                // `groups` map if the group is declared inline.
                let members = if let Some(resolver) = &self.group_resolver {
                    resolver.members(group)
                } else if self.policy.groups.contains_key(group) {
                    self.policy.resolve_group(group).unwrap_or_default()
                } else {
                    return Err(AclError::UnsupportedTargetInPosition(format!(
                        "group '{group}' in dst position requires a GroupResolver; \
                         call AclEvaluator::with_group_resolver or evaluate via AclPolicy::evaluate_permissive"
                    )));
                };
                match &ctx.dst_user {
                    Some(u) => members.iter().any(|m| m == u),
                    None => false,
                }
            }
            AclTarget::Autogroup(name) => match name.as_str() {
                "internet" => true,
                "self" => ctx.src_ip == ctx.dst_ip,
                _ => false,
            },
        };

        Ok(host_matches && dst.ports.contains(ctx.dst_port))
    }
}

impl AclPolicy {
    /// Evaluate whether a connection described by `ctx` is allowed by this policy.
    ///
    /// Rules are evaluated in order. The first matching rule wins.
    /// If no rule matches, the connection is denied (default deny).
    ///
    /// This method is permissive with unresolvable constructs: if a rule
    /// references a `group:` target in destination position and no resolver
    /// is set and the group is not declared in-policy, that rule is skipped.
    /// For strict evaluation that surfaces such errors, use
    /// [`AclEvaluator::try_evaluate`].
    pub fn evaluate(&self, ctx: &EvalContext) -> AclEvalResult {
        self.evaluate_permissive(ctx)
    }

    /// Permissive evaluation: unsupported dst targets behave as non-matching.
    pub fn evaluate_permissive(&self, ctx: &EvalContext) -> AclEvalResult {
        let evaluator = AclEvaluator::new(self);
        match evaluator.try_evaluate(ctx) {
            Ok(r) => r,
            Err(_) => AclEvalResult::default_deny(),
        }
    }

    /// Check if a source target matches the evaluation context.
    pub(crate) fn matches_source(&self, target: &AclTarget, ctx: &EvalContext) -> bool {
        match target {
            AclTarget::Any => true,
            AclTarget::User(user) => ctx.src_user.as_deref() == Some(user.as_str()),
            AclTarget::Group(group) => {
                if ctx.src_groups.contains(group) {
                    return true;
                }
                if let Some(user) = &ctx.src_user
                    && let Ok(members) = self.resolve_group(group)
                {
                    return members.contains(user);
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
}

/// Decision returned by the SSH evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshDecision {
    /// Accept unconditionally.
    Accept,
    /// Accept after an out-of-band check (re-auth, second factor).
    Check,
    /// Default-deny: no rule matched.
    Deny,
}

/// Context for evaluating SSH rules. SSH rules deal in users (the local
/// unix account being logged into) in addition to src/dst.
#[derive(Debug, Clone)]
pub struct SshEvalContext {
    /// The identity of the caller (user email, same as ACL eval).
    pub src_user: Option<String>,
    /// Groups the caller belongs to.
    pub src_groups: Vec<String>,
    /// Source device mesh IP.
    pub src_ip: IpAddr,
    /// Destination device mesh IP.
    pub dst_ip: IpAddr,
    /// Tags assigned to the destination device.
    pub dst_tags: Vec<String>,
    /// The unix account name the caller is trying to log into
    /// ("root", "alice", ...).
    pub ssh_user: String,
}

/// Evaluator for the `sshRules` section of the policy.
pub struct SshAclEvaluator<'a> {
    policy: &'a AclPolicy,
}

impl<'a> SshAclEvaluator<'a> {
    pub fn new(policy: &'a AclPolicy) -> Self {
        Self { policy }
    }

    /// Walk `ssh_rules` in order; the first matching rule wins. Returns
    /// `Deny` when no rule matches.
    pub fn evaluate(&self, ctx: &SshEvalContext) -> SshDecision {
        for rule in &self.policy.ssh {
            let src_matches = rule.src.iter().any(|s| {
                let target = parse_target(s);
                let ctx_eq = EvalContext {
                    src_user: ctx.src_user.clone(),
                    src_groups: ctx.src_groups.clone(),
                    src_ip: ctx.src_ip,
                    ..EvalContext::default()
                };
                self.policy.matches_source(&target, &ctx_eq)
            });
            if !src_matches {
                continue;
            }

            let dst_matches = rule.dst.iter().any(|d| {
                let target = parse_target(d);
                match target {
                    AclTarget::Any => true,
                    AclTarget::Tag(tag) => ctx.dst_tags.iter().any(|t| t == &tag),
                    AclTarget::Ip(ip) => ctx.dst_ip == ip,
                    AclTarget::Cidr(net) => net.contains(&ctx.dst_ip),
                    AclTarget::HostAlias(name) => self
                        .policy
                        .resolve_host(name.as_str())
                        .map(|net| net.contains(&ctx.dst_ip))
                        .unwrap_or(false),
                    _ => false,
                }
            });
            if !dst_matches {
                continue;
            }

            let user_matches = rule.users.iter().any(|u: &String| {
                u == "*"
                    || u.eq_ignore_ascii_case(&ctx.ssh_user)
                    || (u == "autogroup:nonroot" && ctx.ssh_user != "root")
            });
            if !user_matches {
                continue;
            }

            return match rule.action {
                SshAction::Accept => SshDecision::Accept,
                SshAction::Check => SshDecision::Check,
            };
        }
        SshDecision::Deny
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
            dst_user: None,
            proto: None,
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

    #[test]
    fn group_in_dst_without_resolver_errors() {
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["group:devices:*"]}
            ]
        }"#,
        )
        .unwrap();

        let evaluator = AclEvaluator::new(&policy);
        let c = ctx("user@ex.com", &[], "100.64.0.2", 80);
        let err = evaluator.try_evaluate(&c);
        assert!(matches!(err, Err(AclError::UnsupportedTargetInPosition(_))));
    }

    #[test]
    fn group_in_dst_resolves_via_policy_groups() {
        // If the group is declared in-policy it is resolved from there, no
        // external resolver required.
        let policy = AclPolicy::from_json(
            r#"{
            "groups": {"group:admins": ["admin@ex.com"]},
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["group:admins:22"]}
            ]
        }"#,
        )
        .unwrap();

        let evaluator = AclEvaluator::new(&policy);
        let mut c = ctx("user@ex.com", &[], "100.64.0.2", 22);
        c.dst_user = Some("admin@ex.com".to_string());
        let res = evaluator.try_evaluate(&c).unwrap();
        assert!(res.allowed);
    }

    #[test]
    fn group_in_dst_with_external_resolver() {
        struct MockResolver;
        impl GroupResolver for MockResolver {
            fn members(&self, group: &str) -> Vec<String> {
                if group == "group:ops" {
                    vec!["op@ex.com".into()]
                } else {
                    vec![]
                }
            }
        }

        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["group:ops:22"]}
            ]
        }"#,
        )
        .unwrap();

        let evaluator = AclEvaluator::new(&policy).with_group_resolver(Arc::new(MockResolver));
        let mut c = ctx("user@ex.com", &[], "100.64.0.2", 22);
        c.dst_user = Some("op@ex.com".to_string());
        let res = evaluator.try_evaluate(&c).unwrap();
        assert!(res.allowed);

        c.dst_user = Some("someone@ex.com".to_string());
        let res = evaluator.try_evaluate(&c).unwrap();
        assert!(!res.allowed);
    }

    #[test]
    fn proto_filter_matches_when_rule_and_ctx_agree() {
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["*:*"], "proto": "tcp"}
            ]
        }"#,
        )
        .unwrap();
        let evaluator = AclEvaluator::new(&policy);
        let mut c = ctx("u@ex.com", &[], "100.64.0.2", 80);
        c.proto = Some(Proto::Tcp);
        assert!(evaluator.try_evaluate(&c).unwrap().allowed);
        c.proto = Some(Proto::Udp);
        assert!(!evaluator.try_evaluate(&c).unwrap().allowed);
    }

    #[test]
    fn proto_unset_on_rule_is_wildcard() {
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["*:*"]}
            ]
        }"#,
        )
        .unwrap();
        let evaluator = AclEvaluator::new(&policy);
        let mut c = ctx("u@ex.com", &[], "100.64.0.2", 80);
        c.proto = Some(Proto::Udp);
        assert!(evaluator.try_evaluate(&c).unwrap().allowed);
    }

    #[test]
    fn ssh_rule_accept_matches_user_wildcard() {
        let policy = AclPolicy::from_json(
            r#"{
            "ssh": [
                {
                    "action": "accept",
                    "src": ["group:ops"],
                    "dst": ["tag:server"],
                    "users": ["*"]
                }
            ],
            "groups": {"group:ops": ["op@ex.com"]}
        }"#,
        )
        .unwrap();
        let evaluator = SshAclEvaluator::new(&policy);
        let c = SshEvalContext {
            src_user: Some("op@ex.com".into()),
            src_groups: vec!["group:ops".into()],
            src_ip: "100.64.0.1".parse().unwrap(),
            dst_ip: "100.64.0.9".parse().unwrap(),
            dst_tags: vec!["tag:server".into()],
            ssh_user: "root".into(),
        };
        assert_eq!(evaluator.evaluate(&c), SshDecision::Accept);
    }

    #[test]
    fn ssh_rule_check_and_default_deny() {
        let policy = AclPolicy::from_json(
            r#"{
            "ssh": [
                {
                    "action": "check",
                    "src": ["group:ops"],
                    "dst": ["tag:server"],
                    "users": ["autogroup:nonroot"]
                }
            ],
            "groups": {"group:ops": ["op@ex.com"]}
        }"#,
        )
        .unwrap();
        let evaluator = SshAclEvaluator::new(&policy);
        let mut c = SshEvalContext {
            src_user: Some("op@ex.com".into()),
            src_groups: vec!["group:ops".into()],
            src_ip: "100.64.0.1".parse().unwrap(),
            dst_ip: "100.64.0.9".parse().unwrap(),
            dst_tags: vec!["tag:server".into()],
            ssh_user: "alice".into(),
        };
        assert_eq!(evaluator.evaluate(&c), SshDecision::Check);

        // root is blocked by autogroup:nonroot.
        c.ssh_user = "root".into();
        assert_eq!(evaluator.evaluate(&c), SshDecision::Deny);

        // unrelated src.
        c.src_user = Some("stranger@ex.com".into());
        c.src_groups = vec![];
        c.ssh_user = "alice".into();
        assert_eq!(evaluator.evaluate(&c), SshDecision::Deny);
    }

    #[test]
    fn permissive_evaluate_silently_denies_unresolved_group() {
        // AclPolicy::evaluate (permissive) continues to return allowed=false
        // for unresolved groups, preserving backward compatibility.
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["group:unknown:*"]}
            ]
        }"#,
        )
        .unwrap();
        let c = ctx("user@ex.com", &[], "100.64.0.2", 80);
        let result = policy.evaluate(&c);
        assert!(!result.allowed);
    }
}
