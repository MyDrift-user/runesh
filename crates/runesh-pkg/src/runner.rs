//! Command runner for package manager operations.

use crate::{PackageResult, PkgError};

/// Validate a package name's character set to prevent command injection.
///
/// Allows: alphanumeric, hyphens, dots, underscores, forward slashes, @, plus
/// signs, colons. The empty string is accepted as a placeholder (for example,
/// `upgrade_all` may pass `""`). Use [`require_package_name`] when empty is
/// not acceptable.
pub fn validate_package_name(name: &str) -> Result<(), PkgError> {
    if name.is_empty() {
        return Ok(());
    }
    if name.len() > 256 {
        return Err(PkgError::CommandFailed("package name too long".into()));
    }
    for c in name.chars() {
        if !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '/' | '@' | '+' | ':')
        {
            return Err(PkgError::CommandFailed(format!(
                "invalid character '{c}' in package name '{name}'"
            )));
        }
    }
    Ok(())
}

/// Validate a package name and require it to be non-empty. Intended for
/// `install`, `remove`, and the single-package form of `upgrade`.
pub fn require_package_name(name: &str) -> Result<(), PkgError> {
    if name.is_empty() {
        return Err(PkgError::CommandFailed(
            "package name must not be empty".into(),
        ));
    }
    validate_package_name(name)
}

/// Run a package manager command and capture output.
pub async fn run_pkg_command(
    command: &str,
    args: &[&str],
    action: &str,
    package: &str,
) -> Result<PackageResult, PkgError> {
    run_pkg_command_env(command, args, action, package, &[]).await
}

/// Run a package manager command with extra environment overrides (useful for
/// forcing locales, for example `LC_ALL=C` on winget so column-based parsing
/// is stable).
pub async fn run_pkg_command_env(
    command: &str,
    args: &[&str],
    action: &str,
    package: &str,
    env: &[(&str, &str)],
) -> Result<PackageResult, PkgError> {
    validate_package_name(package)?;

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| PkgError::CommandFailed(format!("{command}: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    };

    Ok(PackageResult {
        success: output.status.success(),
        package: package.to_string(),
        action: action.to_string(),
        output: combined,
        exit_code: output.status.code().unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_echo() {
        let cmd = if cfg!(windows) { "cmd" } else { "echo" };
        let args: Vec<&str> = if cfg!(windows) {
            vec!["/C", "echo", "test"]
        } else {
            vec!["test"]
        };
        let result = run_pkg_command(cmd, &args, "test", "test-pkg")
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("test"));
    }

    #[test]
    fn require_package_name_rejects_empty() {
        assert!(require_package_name("").is_err());
        assert!(require_package_name("ok").is_ok());
    }
}
