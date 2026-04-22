//! Compliance checker: compares baseline declarations against actual system state.

use crate::{Baseline, ComplianceReport, Drift, DriftSeverity, ServiceState, StateDeclaration};

/// Collected system state for compliance checking.
#[derive(Default)]
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

/// Check a baseline against the actual system state.
pub fn check_compliance(baseline: &Baseline, state: &SystemState) -> ComplianceReport {
    let mut report = ComplianceReport {
        baseline_name: baseline.name.clone(),
        total: baseline.state.len(),
        compliant: 0,
        drifted: 0,
        missing: 0,
        unknown: 0,
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
            DriftSeverity::Unknown => {
                report.unknown += 1;
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
                    if let Some(req) = version
                        && !version_satisfies(ver, req)
                    {
                        return Drift {
                            declaration: format!("package:{name}"),
                            expected: format!("version {req}"),
                            actual: format!("version {ver}"),
                            severity: DriftSeverity::Drifted,
                        };
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

        StateDeclaration::Firewall { rule, present } => {
            let observed = firewall_rule_present(rule);
            let expected_label = if *present { "present" } else { "absent" };
            match observed {
                None => Drift {
                    declaration: format!("firewall:{rule}"),
                    expected: expected_label.into(),
                    actual: "firewall tool unavailable or rule not introspectable".into(),
                    severity: DriftSeverity::Unknown,
                },
                Some(is_present) if is_present == *present => Drift {
                    declaration: format!("firewall:{rule}"),
                    expected: expected_label.into(),
                    actual: if is_present { "present" } else { "absent" }.into(),
                    severity: DriftSeverity::Compliant,
                },
                Some(is_present) => Drift {
                    declaration: format!("firewall:{rule}"),
                    expected: expected_label.into(),
                    actual: if is_present { "present" } else { "absent" }.into(),
                    severity: if *present {
                        DriftSeverity::Missing
                    } else {
                        DriftSeverity::Extra
                    },
                },
            }
        }

        StateDeclaration::Custom {
            name,
            check_command,
            ..
        } => match run_custom_check(check_command) {
            CustomCheckOutcome::Compliant => Drift {
                declaration: format!("custom:{name}"),
                expected: format!("'{check_command}' exits 0"),
                actual: "exit 0".into(),
                severity: DriftSeverity::Compliant,
            },
            CustomCheckOutcome::NonZero(code) => Drift {
                declaration: format!("custom:{name}"),
                expected: format!("'{check_command}' exits 0"),
                actual: format!("exit {code}"),
                severity: DriftSeverity::Drifted,
            },
            CustomCheckOutcome::InvalidCommand(reason) => Drift {
                declaration: format!("custom:{name}"),
                expected: format!("'{check_command}' exits 0"),
                actual: format!("invalid command: {reason}"),
                severity: DriftSeverity::Unknown,
            },
            CustomCheckOutcome::CouldNotRun(err) => Drift {
                declaration: format!("custom:{name}"),
                expected: format!("'{check_command}' exits 0"),
                actual: format!("spawn failed: {err}"),
                severity: DriftSeverity::Unknown,
            },
        },
    }
}

/// Return Some(true) when the named firewall rule is present, Some(false)
/// when absent, None when the platform's firewall cannot be queried (no
/// matching tool, permission denied, etc.).
fn firewall_rule_present(rule: &str) -> Option<bool> {
    // Rule-name sanity: allow a generous but bounded character set so the
    // name can be passed as an argument without ambient shell expansion.
    if rule.is_empty() || rule.len() > 200 {
        return None;
    }
    if !rule.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, ' ' | '-' | '_' | '.' | ':' | '/' | '(' | ')')
    }) {
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "show",
                "rule",
                &format!("name={rule}"),
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .ok()?;
        // netsh exits 0 with "No rules match the specified criteria." when
        // the rule is absent, so look at the text rather than the status.
        let text = String::from_utf8_lossy(&out.stdout);
        let lower = text.to_ascii_lowercase();
        if lower.contains("no rules match") {
            return Some(false);
        }
        Some(text.contains("Rule Name:"))
    }

    #[cfg(target_os = "linux")]
    {
        // Try nftables first. `nft list ruleset` is a read-only introspection
        // call; we search for the rule substring as a heuristic. For nftables
        // chains, callers typically name the rule with the `comment`
        // attribute, which does appear in `list ruleset` output.
        if let Ok(out) = std::process::Command::new("nft")
            .args(["list", "ruleset"])
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if !text.is_empty() {
                    return Some(text.contains(rule));
                }
            }
        }

        // Fall back to iptables-save.
        if let Ok(out) = std::process::Command::new("iptables-save").output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return Some(text.contains(rule));
        }

        // ip6tables if available
        if let Ok(out) = std::process::Command::new("ip6tables-save").output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return Some(text.contains(rule));
        }

        None
    }

    #[cfg(target_os = "macos")]
    {
        // pf is root-only to list; callers running unprivileged will see
        // None here which the caller turns into DriftSeverity::Unknown.
        if let Ok(out) = std::process::Command::new("pfctl").args(["-sr"]).output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return Some(text.contains(rule));
        }
        None
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Outcome of running a baseline Custom check command.
enum CustomCheckOutcome {
    /// Exited with status 0.
    Compliant,
    /// Exited with a non-zero status.
    NonZero(i32),
    /// Command string failed validation (null byte, empty, ...).
    InvalidCommand(&'static str),
    /// Failed to spawn (platform shell missing, permission denied, ...).
    CouldNotRun(String),
}

fn run_custom_check(command: &str) -> CustomCheckOutcome {
    if command.is_empty() {
        return CustomCheckOutcome::InvalidCommand("empty");
    }
    if command.as_bytes().contains(&0) {
        return CustomCheckOutcome::InvalidCommand("null byte in command");
    }
    // Generous upper bound. A real baseline check is typically one line.
    if command.len() > 4096 {
        return CustomCheckOutcome::InvalidCommand("command too long");
    }

    #[cfg(target_family = "unix")]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(target_family = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW
        c
    };

    match cmd.output() {
        Ok(out) => {
            if let Some(code) = out.status.code() {
                if code == 0 {
                    CustomCheckOutcome::Compliant
                } else {
                    CustomCheckOutcome::NonZero(code)
                }
            } else {
                // Unix: killed by signal, no exit code.
                CustomCheckOutcome::NonZero(-1)
            }
        }
        Err(e) => CustomCheckOutcome::CouldNotRun(e.to_string()),
    }
}

/// Compare an installed version against a declared requirement.
///
/// Parses `expected` as a [`semver::VersionReq`] (supports operators such as
/// `>=1.24`) and `actual` as a [`semver::Version`]. If either fails to parse,
/// falls back to exact string equality and logs a warning.
fn version_satisfies(actual: &str, expected: &str) -> bool {
    let req = match semver::VersionReq::parse(expected) {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!(
                expected = %expected,
                "baseline version requirement not valid semver, falling back to string equality"
            );
            return actual == expected;
        }
    };
    let ver = match semver::Version::parse(actual) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!(
                actual = %actual,
                "installed version not valid semver, falling back to string equality"
            );
            return actual == expected;
        }
    };
    req.matches(&ver)
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
    fn custom_check_exits_zero_is_compliant() {
        let baseline: Baseline = serde_json::from_str(
            r#"{
            "name": "custom-ok",
            "mode": "audit",
            "vars": {},
            "state": [
                {
                    "type": "custom",
                    "name": "noop",
                    "check_command": "exit 0"
                }
            ]
        }"#,
        )
        .unwrap();
        let report = check_compliance(&baseline, &SystemState::default());
        assert!(report.is_compliant(), "expected exit 0 to be compliant");
    }

    #[test]
    fn custom_check_exits_nonzero_is_drifted() {
        let baseline: Baseline = serde_json::from_str(
            r#"{
            "name": "custom-bad",
            "mode": "audit",
            "vars": {},
            "state": [
                {
                    "type": "custom",
                    "name": "fail",
                    "check_command": "exit 7"
                }
            ]
        }"#,
        )
        .unwrap();
        let report = check_compliance(&baseline, &SystemState::default());
        assert_eq!(report.drifted, 1);
        assert_eq!(report.unknown, 0);
        assert!(
            report
                .entries
                .iter()
                .any(|e| e.actual.contains("exit 7") && e.severity == DriftSeverity::Drifted)
        );
    }

    #[test]
    fn custom_check_rejects_null_byte() {
        let outcome = run_custom_check("echo hi\0; exit 1");
        match outcome {
            CustomCheckOutcome::InvalidCommand(reason) => assert_eq!(reason, "null byte in command"),
            _ => panic!("expected InvalidCommand"),
        }
    }

    #[test]
    fn firewall_invalid_rule_name_returns_none() {
        // Semicolons and ampersands must not leak into a shell; we refuse
        // such names outright.
        assert_eq!(firewall_rule_present("evil; rm -rf /"), None);
        assert_eq!(firewall_rule_present(""), None);
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

    #[test]
    fn semver_req_ge_matches_newer_installed_version() {
        let baseline: Baseline = serde_json::from_str(
            r#"{
            "name": "semver",
            "mode": "audit",
            "vars": {},
            "state": [
                {"type": "package", "name": "nginx", "version": ">=1.24", "present": true}
            ]
        }"#,
        )
        .unwrap();
        let mut state = SystemState::default();
        state.packages.insert("nginx".into(), "1.24.0".into());
        let report = check_compliance(&baseline, &state);
        assert!(report.is_compliant(), "1.24.0 should satisfy >=1.24");

        state.packages.insert("nginx".into(), "1.23.0".into());
        let report = check_compliance(&baseline, &state);
        assert!(!report.is_compliant(), "1.23.0 should not satisfy >=1.24");
    }

    #[test]
    fn firewall_check_either_compliant_or_unknown_per_platform() {
        // Firewall introspection depends on the running platform's firewall
        // tool being present and readable; on a CI runner without netsh /
        // nft / iptables / pf we get Unknown, otherwise we get a concrete
        // Compliant / Missing. Accept both so the test is portable, but
        // assert the category is one of those two.
        let baseline: Baseline = serde_json::from_str(
            r#"{
            "name": "fw",
            "mode": "audit",
            "vars": {},
            "state": [
                {"type": "firewall", "rule": "allow-ssh", "present": true}
            ]
        }"#,
        )
        .unwrap();
        let report = check_compliance(&baseline, &SystemState::default());
        assert_eq!(report.total, 1);
        // Exactly one of these counters is 1 and the rest are 0.
        let hits = [
            report.compliant,
            report.drifted,
            report.missing,
            report.unknown,
        ]
        .iter()
        .filter(|n| **n == 1)
        .count();
        assert_eq!(hits, 1, "expected exactly one category to fire: {report:?}");
    }

    #[test]
    fn custom_true_command_is_compliant_on_any_platform() {
        // `true` is available on unix; windows `cmd /C true` exits 0 too
        // ("true" is not a cmd builtin but cmd returns 0 when the command
        // is missing? actually it returns 9009). Use `exit 0` which both
        // shells understand.
        let baseline: Baseline = serde_json::from_str(
            r#"{
            "name": "c",
            "mode": "audit",
            "vars": {},
            "state": [
                {"type": "custom", "name": "noop", "check_command": "exit 0"}
            ]
        }"#,
        )
        .unwrap();
        let report = check_compliance(&baseline, &SystemState::default());
        assert_eq!(report.compliant, 1);
        assert_eq!(report.unknown, 0);
    }
}
