//! Apply remediation for a drifted baseline declaration.
//!
//! The library exposes a [`Remediator`] trait and a [`StdRemediator`]
//! that shells out to the platform's own tools:
//!
//! - Service state: `systemctl` / `sc` / `launchctl`.
//! - File content: `std::fs` write + best-effort mode/owner on Unix.
//! - Setting: `sysctl -w` (Linux) or the registry writer the caller
//!   plugs in (Windows) via [`Remediator`] composition.
//!
//! The library does NOT remediate packages (delegate to
//! `runesh-pkg::PackageManager::install/remove`), users, or firewall
//! rules. Those are domain-specific decisions that belong at the
//! consumer layer because they involve identity provisioning, rule
//! ordering, and side-effects the library cannot observe.
//!
//! Enforcement mode scheduling (audit / notify / enforce) lives in the
//! consumer. This module just supplies a callable primitive.

use std::process::Command;

use crate::{ServiceState, StateDeclaration};

/// Outcome of attempting to remediate one declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediationOutcome {
    /// The system was already in the desired state; no action taken.
    NoChange,
    /// Remediation succeeded; the system should now match the
    /// declaration. Callers may want to re-run drift detection to
    /// confirm.
    Applied,
    /// The declaration kind is outside the library's scope. The caller
    /// must handle it (packages via runesh-pkg, users + firewall at the
    /// consumer layer).
    OutOfScope { kind: &'static str, reason: String },
    /// The remediation failed; the system is likely still drifted.
    Failed(String),
}

/// Remediator for a single [`StateDeclaration`].
///
/// Implementations should be idempotent and side-effect-minimal: if the
/// system already matches, return [`RemediationOutcome::NoChange`].
pub trait Remediator: Send + Sync {
    /// Attempt to bring the system into the declared state.
    fn remediate(&self, decl: &StateDeclaration) -> RemediationOutcome;
}

/// Platform-default remediator. Covers Service / File / Setting and
/// forwards the rest to `OutOfScope`.
pub struct StdRemediator;

impl StdRemediator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdRemediator {
    fn default() -> Self {
        Self::new()
    }
}

impl Remediator for StdRemediator {
    fn remediate(&self, decl: &StateDeclaration) -> RemediationOutcome {
        match decl {
            StateDeclaration::Service {
                name,
                state,
                enabled,
            } => remediate_service(name, *state, *enabled),

            StateDeclaration::File {
                path,
                content,
                mode,
                owner,
                present,
            } => remediate_file(path, content.as_deref(), mode.as_deref(), owner.as_deref(), *present),

            StateDeclaration::Setting { key, value } => {
                let v = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                remediate_setting(key, &v)
            }

            StateDeclaration::Package { .. } => RemediationOutcome::OutOfScope {
                kind: "package",
                reason: "packages are remediated via runesh_pkg::PackageManager at the consumer layer".into(),
            },
            StateDeclaration::User { .. } => RemediationOutcome::OutOfScope {
                kind: "user",
                reason: "user provisioning belongs to the identity plane".into(),
            },
            StateDeclaration::Firewall { .. } => RemediationOutcome::OutOfScope {
                kind: "firewall",
                reason: "firewall rule authoring is consumer-specific and rule-order sensitive".into(),
            },
            StateDeclaration::Custom { .. } => RemediationOutcome::OutOfScope {
                kind: "custom",
                reason: "custom declarations carry their own fix_command; run it via runesh_jobs with an allowlist".into(),
            },
        }
    }
}

// ---- Service ---------------------------------------------------------------

fn remediate_service(
    name: &str,
    desired: ServiceState,
    enabled: bool,
) -> RemediationOutcome {
    if !valid_service_name(name) {
        return RemediationOutcome::Failed(format!("invalid service name: {name}"));
    }

    #[cfg(target_os = "linux")]
    {
        let (state_action, enable_action) = match (desired, enabled) {
            (ServiceState::Running, true) => ("start", Some("enable")),
            (ServiceState::Running, false) => ("start", Some("disable")),
            (ServiceState::Stopped, true) => ("stop", Some("enable")),
            (ServiceState::Stopped, false) => ("stop", Some("disable")),
        };
        if let Err(e) = run_ok(Command::new("systemctl").args([state_action, name])) {
            return RemediationOutcome::Failed(format!("systemctl {state_action} {name}: {e}"));
        }
        if let Some(act) = enable_action
            && let Err(e) = run_ok(Command::new("systemctl").args([act, name]))
        {
            return RemediationOutcome::Failed(format!("systemctl {act} {name}: {e}"));
        }
        return RemediationOutcome::Applied;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = enabled; // launchctl load/unload handles both state and persistence.
        let cmd = match desired {
            ServiceState::Running => "load",
            ServiceState::Stopped => "unload",
        };
        match run_ok(Command::new("launchctl").args([cmd, name])) {
            Ok(()) => RemediationOutcome::Applied,
            Err(e) => RemediationOutcome::Failed(format!("launchctl {cmd} {name}: {e}")),
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // sc start/stop is synchronous enough for drift remediation.
        // `sc config start= auto|demand` sets enable/disable.
        let state_action = match desired {
            ServiceState::Running => "start",
            ServiceState::Stopped => "stop",
        };
        let mut state_cmd = Command::new("sc");
        state_cmd
            .args([state_action, name])
            .creation_flags(0x08000000);
        if let Err(e) = run_ok(&mut state_cmd) {
            // sc returns non-zero when the service is already running /
            // stopped; that's fine. We surface the error only when the
            // stdout doesn't contain the "already" marker.
            let lower = e.to_ascii_lowercase();
            if !lower.contains("already") && !lower.contains("1056") && !lower.contains("1062") {
                return RemediationOutcome::Failed(format!("sc {state_action} {name}: {e}"));
            }
        }

        let start_type = if enabled { "auto" } else { "demand" };
        let mut cfg_cmd = Command::new("sc");
        cfg_cmd
            .args(["config", name, &format!("start= {start_type}")])
            .creation_flags(0x08000000);
        if let Err(e) = run_ok(&mut cfg_cmd) {
            return RemediationOutcome::Failed(format!(
                "sc config {name} start= {start_type}: {e}"
            ));
        }
        return RemediationOutcome::Applied;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (desired, enabled, name);
        RemediationOutcome::OutOfScope {
            kind: "service",
            reason: "no service manager integration for this platform".into(),
        }
    }
}

fn valid_service_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':'))
}

// ---- File -----------------------------------------------------------------

fn remediate_file(
    path: &str,
    content: Option<&str>,
    mode: Option<&str>,
    owner: Option<&str>,
    present: bool,
) -> RemediationOutcome {
    let p = std::path::Path::new(path);

    if !present {
        // Absent: remove the file if it exists.
        return match std::fs::remove_file(p) {
            Ok(()) => RemediationOutcome::Applied,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => RemediationOutcome::NoChange,
            Err(e) => RemediationOutcome::Failed(format!("remove {path}: {e}")),
        };
    }

    // Present: make sure the file exists and (if requested) has the
    // expected content. Directory components are created as needed.
    if let Some(parent) = p.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return RemediationOutcome::Failed(format!("mkdir -p {}: {e}", parent.display()));
    }

    let mut changed = false;
    match content {
        Some(want) => {
            let current = std::fs::read_to_string(p).ok();
            if current.as_deref() != Some(want) {
                if let Err(e) = std::fs::write(p, want) {
                    return RemediationOutcome::Failed(format!("write {path}: {e}"));
                }
                changed = true;
            }
        }
        None => {
            if !p.exists() {
                if let Err(e) = std::fs::File::create(p) {
                    return RemediationOutcome::Failed(format!("create {path}: {e}"));
                }
                changed = true;
            }
        }
    }

    #[cfg(unix)]
    {
        if let Some(m) = mode
            && let Some(parsed) = parse_octal_mode(m)
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(parsed);
            if let Err(e) = std::fs::set_permissions(p, perms) {
                return RemediationOutcome::Failed(format!("chmod {path}: {e}"));
            }
        }
        if let Some(o) = owner {
            // `chown` shell-out keeps the library dep-free; group can be
            // specified as user:group, matching the system tool.
            if !o
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
            {
                return RemediationOutcome::Failed(format!("invalid owner: {o}"));
            }
            if let Err(e) = run_ok(Command::new("chown").args([o, path])) {
                return RemediationOutcome::Failed(format!("chown {o} {path}: {e}"));
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (mode, owner);
    }

    if changed {
        RemediationOutcome::Applied
    } else {
        RemediationOutcome::NoChange
    }
}

#[cfg(unix)]
fn parse_octal_mode(s: &str) -> Option<u32> {
    let trimmed = s.trim_start_matches("0o").trim_start_matches('0');
    if trimmed.is_empty() {
        return Some(0);
    }
    u32::from_str_radix(trimmed, 8).ok()
}

// ---- Setting --------------------------------------------------------------

fn remediate_setting(key: &str, value: &str) -> RemediationOutcome {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
        return RemediationOutcome::Failed(format!("invalid setting key: {key}"));
    }

    #[cfg(target_os = "linux")]
    {
        // sysctl -w net.ipv4.ip_forward=1
        let arg = format!("{key}={value}");
        return match run_ok(Command::new("sysctl").args(["-w", &arg])) {
            Ok(()) => RemediationOutcome::Applied,
            Err(e) => RemediationOutcome::Failed(format!("sysctl -w {arg}: {e}")),
        };
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = value;
        RemediationOutcome::OutOfScope {
            kind: "setting",
            reason: "non-Linux setting writes require a platform-specific writer (Windows registry, macOS defaults); supply a custom Remediator".into(),
        }
    }
}

// ---- Helpers --------------------------------------------------------------

fn run_ok(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_user_firewall_are_out_of_scope() {
        let r = StdRemediator::new();
        let outcomes = [
            r.remediate(&StateDeclaration::Package {
                name: "nginx".into(),
                version: None,
                present: true,
            }),
            r.remediate(&StateDeclaration::User {
                name: "bob".into(),
                groups: vec![],
                present: true,
            }),
            r.remediate(&StateDeclaration::Firewall {
                rule: "allow-ssh".into(),
                present: true,
            }),
        ];
        for o in outcomes {
            match o {
                RemediationOutcome::OutOfScope { .. } => {}
                other => panic!("expected OutOfScope, got {other:?}"),
            }
        }
    }

    #[test]
    fn custom_is_out_of_scope_with_pointer() {
        let r = StdRemediator::new();
        let o = r.remediate(&StateDeclaration::Custom {
            name: "bios-locked".into(),
            check_command: "true".into(),
            fix_command: Some("echo fix".into()),
        });
        match o {
            RemediationOutcome::OutOfScope { kind: "custom", reason } => {
                assert!(reason.contains("runesh_jobs"), "pointer to jobs crate: {reason}");
            }
            other => panic!("expected OutOfScope custom, got {other:?}"),
        }
    }

    #[test]
    fn file_remediation_creates_and_removes() {
        let tmp = std::env::temp_dir().join(format!(
            "runesh-baseline-rem-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        let r = StdRemediator::new();

        // Present with content.
        let decl = StateDeclaration::File {
            path: tmp.to_string_lossy().to_string(),
            content: Some("hello".into()),
            mode: None,
            owner: None,
            present: true,
        };
        assert_eq!(r.remediate(&decl), RemediationOutcome::Applied);
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello");
        // Second run: no change.
        assert_eq!(r.remediate(&decl), RemediationOutcome::NoChange);

        // Absent: file removed.
        let decl_absent = StateDeclaration::File {
            path: tmp.to_string_lossy().to_string(),
            content: None,
            mode: None,
            owner: None,
            present: false,
        };
        assert_eq!(r.remediate(&decl_absent), RemediationOutcome::Applied);
        assert!(!tmp.exists());
        // Already gone: no change.
        assert_eq!(r.remediate(&decl_absent), RemediationOutcome::NoChange);
    }

    #[test]
    fn invalid_service_name_rejected() {
        let r = StdRemediator::new();
        let o = r.remediate(&StateDeclaration::Service {
            name: "evil; rm -rf /".into(),
            state: ServiceState::Running,
            enabled: true,
        });
        match o {
            RemediationOutcome::Failed(m) => assert!(m.contains("invalid service name")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn octal_mode_parser() {
        assert_eq!(parse_octal_mode("0644"), Some(0o644));
        assert_eq!(parse_octal_mode("644"), Some(0o644));
        assert_eq!(parse_octal_mode("0o755"), Some(0o755));
        assert_eq!(parse_octal_mode("garbage"), None);
    }
}
