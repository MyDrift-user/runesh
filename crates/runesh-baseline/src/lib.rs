//! Declarative baselines with drift detection.
//!
//! A baseline declares desired state across packages, services, files,
//! firewall rules, users, and other system facets. The engine compares
//! declared state against actual state and reports drift.

pub mod checker;

pub use checker::{SystemState, check_compliance};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A composable baseline definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// Baseline name.
    pub name: String,
    /// Optional parent baseline to inherit from.
    #[serde(default)]
    pub inherits: Option<String>,
    /// Enforcement mode.
    #[serde(default)]
    pub mode: EnforcementMode,
    /// Environment variables available in expressions.
    #[serde(default)]
    pub vars: HashMap<String, String>,
    /// Desired state declarations.
    #[serde(default)]
    pub state: Vec<StateDeclaration>,
}

/// How drift is handled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnforcementMode {
    /// Report drift but take no action.
    #[default]
    Audit,
    /// Report drift and send notifications.
    Notify,
    /// Automatically remediate drift.
    Enforce,
}

/// A single desired-state declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StateDeclaration {
    /// A package must be installed (optionally at a specific version).
    Package {
        name: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// A service must be in a specific state.
    Service {
        name: String,
        #[serde(default = "default_running")]
        state: ServiceState,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    /// A file must exist with specific content or permissions.
    File {
        path: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        owner: Option<String>,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// A firewall rule must exist.
    Firewall {
        rule: String,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// A user must exist with specific properties.
    User {
        name: String,
        #[serde(default)]
        groups: Vec<String>,
        #[serde(default = "default_true")]
        present: bool,
    },
    /// A registry key (Windows) or sysctl (Linux) must have a value.
    Setting {
        key: String,
        value: serde_json::Value,
    },
    /// A custom check command (exit 0 = compliant).
    Custom {
        name: String,
        check_command: String,
        #[serde(default)]
        fix_command: Option<String>,
    },
}

/// Desired service state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    Running,
    Stopped,
}

/// A drift report entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Drift {
    /// Which declaration drifted.
    pub declaration: String,
    /// What was expected.
    pub expected: String,
    /// What was found.
    pub actual: String,
    /// Severity.
    pub severity: DriftSeverity,
}

/// Drift severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriftSeverity {
    /// State matches (no drift).
    Compliant,
    /// State drifted.
    Drifted,
    /// Expected item is missing entirely.
    Missing,
    /// Unexpected item found (not declared).
    Extra,
}

/// Result of evaluating a baseline against actual state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub baseline_name: String,
    pub total: usize,
    pub compliant: usize,
    pub drifted: usize,
    pub missing: usize,
    pub entries: Vec<Drift>,
}

impl ComplianceReport {
    /// Overall compliance percentage.
    pub fn compliance_percent(&self) -> f64 {
        if self.total == 0 {
            return 100.0;
        }
        (self.compliant as f64 / self.total as f64) * 100.0
    }

    /// Whether everything is compliant.
    pub fn is_compliant(&self) -> bool {
        self.drifted == 0 && self.missing == 0
    }
}

impl Baseline {
    /// Merge a parent baseline into this one (inheritance).
    pub fn merge_parent(&mut self, parent: &Baseline) {
        // Parent vars are defaults; child overrides
        for (k, v) in &parent.vars {
            self.vars.entry(k.clone()).or_insert_with(|| v.clone());
        }
        // Prepend parent state (child declarations take precedence)
        let mut merged = parent.state.clone();
        merged.extend(self.state.drain(..));
        self.state = merged;
    }

    /// Substitute ${var} references in string values.
    pub fn resolve_vars(&mut self) {
        let vars = self.vars.clone();
        for decl in &mut self.state {
            match decl {
                StateDeclaration::File { content, .. } => {
                    if let Some(c) = content {
                        *c = substitute(c, &vars);
                    }
                }
                StateDeclaration::Setting { value, .. } => {
                    if let serde_json::Value::String(s) = value {
                        *s = substitute(s, &vars);
                    }
                }
                _ => {}
            }
        }
    }
}

fn substitute(s: &str, vars: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (k, v) in vars {
        result = result.replace(&format!("${{{k}}}"), v);
    }
    result
}

fn default_true() -> bool {
    true
}

fn default_running() -> ServiceState {
    ServiceState::Running
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_baseline() -> Baseline {
        serde_yaml::from_str(
            r#"
name: linux/server
mode: audit
vars:
  dns_server: "10.0.0.1"
state:
  - type: package
    name: nginx
  - type: service
    name: nginx
    state: running
    enabled: true
  - type: file
    path: /etc/resolv.conf
    content: "nameserver ${dns_server}"
  - type: setting
    key: net.ipv4.ip_forward
    value: "1"
  - type: user
    name: deploy
    groups: [sudo, docker]
"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_yaml_baseline() {
        let b = sample_baseline();
        assert_eq!(b.name, "linux/server");
        assert_eq!(b.state.len(), 5);
        assert_eq!(b.mode, EnforcementMode::Audit);
    }

    #[test]
    fn resolve_variables() {
        let mut b = sample_baseline();
        b.resolve_vars();

        if let StateDeclaration::File { content, .. } = &b.state[2] {
            assert_eq!(content.as_deref(), Some("nameserver 10.0.0.1"));
        } else {
            panic!("expected file declaration");
        }
    }

    #[test]
    fn merge_parent_baseline() {
        let parent = Baseline {
            name: "base".into(),
            inherits: None,
            mode: EnforcementMode::Audit,
            vars: HashMap::from([("env".into(), "prod".into())]),
            state: vec![StateDeclaration::Package {
                name: "curl".into(),
                version: None,
                present: true,
            }],
        };

        let mut child = Baseline {
            name: "linux/server".into(),
            inherits: Some("base".into()),
            mode: EnforcementMode::Enforce,
            vars: HashMap::new(),
            state: vec![StateDeclaration::Package {
                name: "nginx".into(),
                version: None,
                present: true,
            }],
        };

        child.merge_parent(&parent);
        assert_eq!(child.state.len(), 2); // curl from parent + nginx from child
        assert_eq!(child.vars["env"], "prod");
    }

    #[test]
    fn compliance_report() {
        let report = ComplianceReport {
            baseline_name: "test".into(),
            total: 10,
            compliant: 8,
            drifted: 1,
            missing: 1,
            entries: vec![],
        };
        assert_eq!(report.compliance_percent(), 80.0);
        assert!(!report.is_compliant());
    }

    #[test]
    fn empty_baseline_is_compliant() {
        let report = ComplianceReport::default();
        assert!(report.is_compliant());
        assert_eq!(report.compliance_percent(), 100.0);
    }

    #[test]
    fn json_roundtrip() {
        let b = sample_baseline();
        let json = serde_json::to_string(&b).unwrap();
        let parsed: Baseline = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "linux/server");
        assert_eq!(parsed.state.len(), 5);
    }

    #[test]
    fn all_enforcement_modes() {
        for mode in [
            EnforcementMode::Audit,
            EnforcementMode::Notify,
            EnforcementMode::Enforce,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let parsed: EnforcementMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, mode);
        }
    }
}
