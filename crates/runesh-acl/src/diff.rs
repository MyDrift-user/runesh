//! ACL diff engine.
//!
//! Compares two ACL policies and reports which active sessions
//! would be affected by the change.

use std::net::IpAddr;

use crate::eval::EvalContext;
use crate::model::AclPolicy;

/// A session that might be affected by an ACL change.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub src_user: Option<String>,
    pub src_groups: Vec<String>,
    pub src_tags: Vec<String>,
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub dst_tags: Vec<String>,
    pub dst_port: u16,
    /// Human-readable label for display.
    pub label: String,
}

impl ActiveSession {
    fn to_eval_context(&self) -> EvalContext {
        EvalContext {
            src_user: self.src_user.clone(),
            src_groups: self.src_groups.clone(),
            src_tags: self.src_tags.clone(),
            src_ip: self.src_ip,
            dst_ip: self.dst_ip,
            dst_tags: self.dst_tags.clone(),
            dst_port: self.dst_port,
            dst_user: None,
            proto: None,
        }
    }
}

/// The effect of an ACL change on a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEffect {
    /// Was allowed, still allowed.
    StillAllowed,
    /// Was denied, still denied.
    StillDenied,
    /// Was allowed, now denied.
    WillDrop,
    /// Was denied, now allowed.
    WillAllow,
}

/// A single entry in an ACL diff result.
#[derive(Debug, Clone)]
pub struct AclDiffEntry {
    pub session: String,
    pub effect: SessionEffect,
    pub old_rule: Option<usize>,
    pub new_rule: Option<usize>,
}

/// Result of diffing two ACL policies against active sessions.
#[derive(Debug, Clone)]
pub struct AclDiff {
    pub entries: Vec<AclDiffEntry>,
}

impl AclDiff {
    /// Compare two ACL policies against a set of active sessions.
    ///
    /// Returns which sessions will be affected and how.
    pub fn compute(old: &AclPolicy, new: &AclPolicy, sessions: &[ActiveSession]) -> Self {
        let entries = sessions
            .iter()
            .filter_map(|session| {
                let ctx = session.to_eval_context();
                let old_result = old.evaluate(&ctx);
                let new_result = new.evaluate(&ctx);

                let effect = match (old_result.allowed, new_result.allowed) {
                    (true, true) => SessionEffect::StillAllowed,
                    (false, false) => SessionEffect::StillDenied,
                    (true, false) => SessionEffect::WillDrop,
                    (false, true) => SessionEffect::WillAllow,
                };

                // Only report changes
                if effect == SessionEffect::StillAllowed || effect == SessionEffect::StillDenied {
                    return None;
                }

                Some(AclDiffEntry {
                    session: session.label.clone(),
                    effect,
                    old_rule: old_result.matching_rule,
                    new_rule: new_result.matching_rule,
                })
            })
            .collect();

        Self { entries }
    }

    /// Count sessions that will be dropped.
    pub fn dropped_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.effect == SessionEffect::WillDrop)
            .count()
    }

    /// Count sessions that will be newly allowed.
    pub fn allowed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.effect == SessionEffect::WillAllow)
            .count()
    }

    /// Returns true if no sessions are affected.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Format a human-readable summary.
    pub fn summary(&self) -> String {
        let dropped = self.dropped_count();
        let allowed = self.allowed_count();
        match (dropped, allowed) {
            (0, 0) => "No sessions affected.".to_string(),
            (d, 0) => format!("{d} sessions will be dropped."),
            (0, a) => format!("{a} sessions will be newly allowed."),
            (d, a) => format!("{d} sessions will be dropped, {a} newly allowed."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AclPolicy;

    #[test]
    fn diff_detects_dropped_sessions() {
        let old = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["*"], "dst": ["*:*"]}
            ]
        }"#,
        )
        .unwrap();

        let new = AclPolicy::from_json(
            r#"{
            "acls": [
                {"action": "accept", "src": ["group:admin"], "dst": ["*:*"]}
            ],
            "groups": {"group:admin": ["admin@ex.com"]}
        }"#,
        )
        .unwrap();

        let sessions = vec![
            ActiveSession {
                src_user: Some("admin@ex.com".into()),
                src_groups: vec![],
                src_tags: vec![],
                src_ip: "100.64.0.1".parse().unwrap(),
                dst_ip: "100.64.0.2".parse().unwrap(),
                dst_tags: vec![],
                dst_port: 22,
                label: "admin -> server:22".into(),
            },
            ActiveSession {
                src_user: Some("user@ex.com".into()),
                src_groups: vec![],
                src_tags: vec![],
                src_ip: "100.64.0.3".parse().unwrap(),
                dst_ip: "100.64.0.2".parse().unwrap(),
                dst_tags: vec![],
                dst_port: 80,
                label: "user -> server:80".into(),
            },
        ];

        let diff = AclDiff::compute(&old, &new, &sessions);
        assert_eq!(diff.dropped_count(), 1);
        assert_eq!(diff.allowed_count(), 0);
        assert_eq!(diff.entries[0].session, "user -> server:80");
        assert_eq!(diff.entries[0].effect, SessionEffect::WillDrop);
    }

    #[test]
    fn diff_detects_newly_allowed() {
        let old = AclPolicy::from_json(r#"{"acls": []}"#).unwrap();
        let new = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();

        let sessions = vec![ActiveSession {
            src_user: Some("user@ex.com".into()),
            src_groups: vec![],
            src_tags: vec![],
            src_ip: "100.64.0.1".parse().unwrap(),
            dst_ip: "100.64.0.2".parse().unwrap(),
            dst_tags: vec![],
            dst_port: 80,
            label: "user -> server:80".into(),
        }];

        let diff = AclDiff::compute(&old, &new, &sessions);
        assert_eq!(diff.allowed_count(), 1);
        assert_eq!(diff.dropped_count(), 0);
    }

    #[test]
    fn no_changes_produces_empty_diff() {
        let policy = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();

        let sessions = vec![ActiveSession {
            src_user: Some("user@ex.com".into()),
            src_groups: vec![],
            src_tags: vec![],
            src_ip: "100.64.0.1".parse().unwrap(),
            dst_ip: "100.64.0.2".parse().unwrap(),
            dst_tags: vec![],
            dst_port: 80,
            label: "user -> server:80".into(),
        }];

        let diff = AclDiff::compute(&policy, &policy, &sessions);
        assert!(diff.is_empty());
    }

    #[test]
    fn summary_format() {
        let old = AclPolicy::from_json(
            r#"{
            "acls": [{"action": "accept", "src": ["*"], "dst": ["*:*"]}]
        }"#,
        )
        .unwrap();
        let new = AclPolicy::from_json(r#"{"acls": []}"#).unwrap();

        let sessions = vec![
            ActiveSession {
                src_user: Some("a@ex.com".into()),
                src_groups: vec![],
                src_tags: vec![],
                src_ip: "100.64.0.1".parse().unwrap(),
                dst_ip: "100.64.0.2".parse().unwrap(),
                dst_tags: vec![],
                dst_port: 80,
                label: "a".into(),
            },
            ActiveSession {
                src_user: Some("b@ex.com".into()),
                src_groups: vec![],
                src_tags: vec![],
                src_ip: "100.64.0.3".parse().unwrap(),
                dst_ip: "100.64.0.4".parse().unwrap(),
                dst_tags: vec![],
                dst_port: 443,
                label: "b".into(),
            },
        ];

        let diff = AclDiff::compute(&old, &new, &sessions);
        assert_eq!(diff.summary(), "2 sessions will be dropped.");
    }
}
