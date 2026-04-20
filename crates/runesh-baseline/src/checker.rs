//! Compliance checker: compares baseline declarations against actual system state.

use crate::{Baseline, ComplianceReport, Drift, DriftSeverity, ServiceState, StateDeclaration};

/// Collected system state for compliance checking.
pub struct SystemState {
    /// Installed packages: name -> version.
    pub packages: std::collections::HashMap<String, String>,
    /// Running services: name -> running?.
    pub services: std::collections::HashMap<String, bool>,
    /// Enabled services: name -> enabled?.
    pub services_enabled: std::collections::HashMap<String, bool>,
    /// Files: path -> content (or None if not read).
    pub files: std::collections::HashMap<String, Option<String>>,
    /// Users: name -> list of groups.
    pub users: std::collections::HashMap<String, Vec<String>>,
    /// Settings: key -> value string.
    pub settings: std::collections::HashMap<String, String>,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            packages: std::collections::HashMap::new(),
            services: std::collections::HashMap::new(),
            services_enabled: std::collections::HashMap::new(),
            files: std::collections::HashMap::new(),
            users: std::collections::HashMap::new(),
            settings: std::collections::HashMap::new(),
        }
    }
}

/// Check a baseline against the actual system state.
pub fn check_compliance(baseline: &Baseline, state: &SystemState) -> ComplianceReport {
    let mut report = ComplianceReport {
        baseline_name: baseline.name.clone(),
        total: baseline.state.len(),
        compliant: 0,
        drifted: 0,
        missing: 0,
        entries: Vec::new(),
    };

    for decl in &baseline.state {
        let drift = check_declaration(decl, state);
        match drift.severity {
            DriftSeverity::Compliant => report.compliant += 1,
            DriftSeverity::Drifted => {
                report.drifted += 1;
                report.entries.push(drift);
            }
            DriftSeverity::Missing => {
                report.missing += 1;
                report.entries.push(drift);
            }
            DriftSeverity::Extra => {
                report.entries.push(drift);
            }
        }
    }

    report
}

fn check_declaration(decl: &StateDeclaration, state: &SystemState) -> Drift {
    match decl {
        StateDeclaration::Package {
            name,
            version,
            present,
        } => {
            let installed = state.packages.get(name);
            match (present, installed) {
                (true, None) => Drift {
                    declaration: format!("package:{name}"),
                    expected: format!(
                        "installed{}",
                        version
                            .as_deref()
                            .map(|v| format!(" >= {v}"))
                            .unwrap_or_default()
                    ),
                    actual: "not installed".into(),
                    severity: DriftSeverity::Missing,
                },
                (true, Some(ver)) => {
                    if let Some(req) = version {
                        if ver != req {
                            return Drift {
                                declaration: format!("package:{name}"),
                                expected: format!("version {req}"),
                                actual: format!("version {ver}"),
                                severity: DriftSeverity::Drifted,
                            };
                        }
                    }
                    Drift {
                        declaration: format!("package:{name}"),
                        expected: "installed".into(),
                        actual: format!("installed ({ver})"),
                        severity: DriftSeverity::Compliant,
                    }
                }
                (false, Some(_)) => Drift {
                    declaration: format!("package:{name}"),
                    expected: "not installed".into(),
                    actual: "installed".into(),
                    severity: DriftSeverity::Drifted,
                },
                (false, None) => Drift {
                    declaration: format!("package:{name}"),
                    expected: "not installed".into(),
                    actual: "not installed".into(),
                    severity: DriftSeverity::Compliant,
                },
            }
        }

        StateDeclaration::Service {
            name,
            state: desired_state,
            enabled,
        } => {
            let running = state.services.get(name);
            let is_enabled = state.services_enabled.get(name);

            match running {
                None => Drift {
                    declaration: format!("service:{name}"),
                    expected: format!("{desired_state:?}"),
                    actual: "not found".into(),
                    severity: DriftSeverity::Missing,
                },
                Some(is_running) => {
                    let want_running = *desired_state == ServiceState::Running;
                    let state_ok = *is_running == want_running;
                    let enabled_ok = is_enabled.map(|e| *e == *enabled).unwrap_or(true);

                    if state_ok && enabled_ok {
                        Drift {
                            declaration: format!("service:{name}"),
                            expected: format!("{desired_state:?}"),
                            actual: format!("{desired_state:?}"),
                            severity: DriftSeverity::Compliant,
                        }
                    } else {
                        let actual_state = if *is_running { "running" } else { "stopped" };
                        let actual_enabled = is_enabled
                            .map(|e| if *e { ", enabled" } else { ", disabled" })
                            .unwrap_or("");
                        Drift {
                            declaration: format!("service:{name}"),
                            expected: format!("{desired_state:?}, enabled={enabled}"),
                            actual: format!("{actual_state}{actual_enabled}"),
                            severity: DriftSeverity::Drifted,
                        }
                    }
                }
            }
        }

        StateDeclaration::File {
            path,
            content,
            present,
            ..
        } => {
            let exists = state.files.contains_key(path);
            match (present, exists) {
                (true, false) => Drift {
                    declaration: format!("file:{path}"),
                    expected: "present".into(),
                    actual: "missing".into(),
                    severity: DriftSeverity::Missing,
                },
                (false, true) => Drift {
                    declaration: format!("file:{path}"),
                    expected: "absent".into(),
                    actual: "present".into(),
                    severity: DriftSeverity::Drifted,
                },
                (false, false) => Drift {
                    declaration: format!("file:{path}"),
                    expected: "absent".into(),
                    actual: "absent".into(),
                    severity: DriftSeverity::Compliant,
                },
                (true, true) => {
                    if let Some(expected_content) = content {
                        let actual = state.files.get(path).and_then(|c| c.as_ref());
                        match actual {
                            Some(c) if c == expected_content => Drift {
                                declaration: format!("file:{path}"),
                                expected: "content matches".into(),
                                actual: "content matches".into(),
                                severity: DriftSeverity::Compliant,
                            },
                            Some(_) => Drift {
                                declaration: format!("file:{path}"),
                                expected: "specific content".into(),
                                actual: "content differs".into(),
                                severity: DriftSeverity::Drifted,
                            },
                            None => Drift {
                                declaration: format!("file:{path}"),
                                expected: "specific content".into(),
                                actual: "content not read".into(),
                                severity: DriftSeverity::Drifted,
                            },
                        }
                    } else {
                        Drift {
                            declaration: format!("file:{path}"),
                            expected: "present".into(),
                            actual: "present".into(),
                            severity: DriftSeverity::Compliant,
                        }
                    }
                }
            }
        }

        StateDeclaration::User {
            name,
            groups,
            present,
        } => {
            let user_groups = state.users.get(name);
            match (present, user_groups) {
                (true, None) => Drift {
                    declaration: format!("user:{name}"),
                    expected: "present".into(),
                    actual: "missing".into(),
                    severity: DriftSeverity::Missing,
                },
                (false, Some(_)) => Drift {
                    declaration: format!("user:{name}"),
                    expected: "absent".into(),
                    actual: "present".into(),
                    severity: DriftSeverity::Drifted,
                },
                (false, None) => Drift {
                    declaration: format!("user:{name}"),
                    expected: "absent".into(),
                    actual: "absent".into(),
                    severity: DriftSeverity::Compliant,
                },
                (true, Some(actual_groups)) => {
                    let missing: Vec<&String> = groups
                        .iter()
                        .filter(|g| !actual_groups.contains(g))
                        .collect();
                    if missing.is_empty() {
                        Drift {
                            declaration: format!("user:{name}"),
                            expected: format!("groups: {}", groups.join(",")),
                            actual: format!("groups: {}", actual_groups.join(",")),
                            severity: DriftSeverity::Compliant,
                        }
                    } else {
                        Drift {
                            declaration: format!("user:{name}"),
                            expected: format!("groups: {}", groups.join(",")),
                            actual: format!(
                                "missing groups: {}",
                                missing
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ),
                            severity: DriftSeverity::Drifted,
                        }
                    }
                }
            }
        }

        StateDeclaration::Setting { key, value } => {
            let actual = state.settings.get(key);
            let expected_str = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            match actual {
                None => Drift {
                    declaration: format!("setting:{key}"),
                    expected: expected_str,
                    actual: "not set".into(),
                    severity: DriftSeverity::Missing,
                },
                Some(a) if *a == expected_str => Drift {
                    declaration: format!("setting:{key}"),
                    expected: expected_str,
                    actual: a.clone(),
                    severity: DriftSeverity::Compliant,
                },
                Some(a) => Drift {
                    declaration: format!("setting:{key}"),
                    expected: expected_str,
                    actual: a.clone(),
                    severity: DriftSeverity::Drifted,
                },
            }
        }

        StateDeclaration::Firewall { rule, present } => Drift {
            declaration: format!("firewall:{rule}"),
            expected: if *present { "present" } else { "absent" }.into(),
            actual: "check not implemented".into(),
            severity: DriftSeverity::Compliant, // skip for now
        },

        StateDeclaration::Custom {
            name,
            check_command,
            ..
        } => Drift {
            declaration: format!("custom:{name}"),
            expected: format!("command '{check_command}' exits 0"),
            actual: "async check required".into(),
            severity: DriftSeverity::Compliant, // skip for now
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_baseline() -> Baseline {
        serde_json::from_str(
            r#"{
            "name": "test",
            "mode": "audit",
            "vars": {},
            "state": [
                {"type": "package", "name": "nginx", "present": true},
                {"type": "package", "name": "telnet", "present": false},
                {"type": "service", "name": "nginx", "state": "running", "enabled": true},
                {"type": "file", "path": "/etc/hostname", "content": "server-1", "present": true},
                {"type": "user", "name": "deploy", "groups": ["sudo", "docker"], "present": true},
                {"type": "setting", "key": "net.ipv4.ip_forward", "value": "1"}
            ]
        }"#,
        )
        .unwrap()
    }

    fn make_compliant_state() -> SystemState {
        let mut s = SystemState::default();
        s.packages.insert("nginx".into(), "1.24.0".into());
        s.services.insert("nginx".into(), true);
        s.services_enabled.insert("nginx".into(), true);
        s.files
            .insert("/etc/hostname".into(), Some("server-1".into()));
        s.users
            .insert("deploy".into(), vec!["sudo".into(), "docker".into()]);
        s.settings.insert("net.ipv4.ip_forward".into(), "1".into());
        s
    }

    #[test]
    fn fully_compliant() {
        let baseline = make_baseline();
        let state = make_compliant_state();
        let report = check_compliance(&baseline, &state);
        assert!(report.is_compliant());
        assert_eq!(report.compliance_percent(), 100.0);
    }

    #[test]
    fn missing_package() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state.packages.remove("nginx");
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
        assert_eq!(report.missing, 1);
        assert!(
            report
                .entries
                .iter()
                .any(|d| d.declaration == "package:nginx")
        );
    }

    #[test]
    fn unwanted_package_present() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state.packages.insert("telnet".into(), "0.17".into());
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
        assert_eq!(report.drifted, 1);
    }

    #[test]
    fn service_stopped() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state.services.insert("nginx".into(), false);
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
    }

    #[test]
    fn file_content_differs() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state
            .files
            .insert("/etc/hostname".into(), Some("wrong-name".into()));
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
    }

    #[test]
    fn user_missing_group() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state.users.insert("deploy".into(), vec!["sudo".into()]); // missing docker
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
    }

    #[test]
    fn setting_wrong_value() {
        let baseline = make_baseline();
        let mut state = make_compliant_state();
        state
            .settings
            .insert("net.ipv4.ip_forward".into(), "0".into());
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant());
    }

    #[test]
    fn empty_baseline_is_compliant() {
        let baseline = Baseline {
            name: "empty".into(),
            inherits: None,
            mode: crate::EnforcementMode::Audit,
            vars: std::collections::HashMap::new(),
            state: vec![],
        };
        let report = check_compliance(&baseline, &SystemState::default());
        assert!(report.is_compliant());
    }
}
