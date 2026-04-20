//! Command runner for package manager operations.

use crate::{PackageResult, PkgError};

/// Validate a package name to prevent command injection.
/// Allows: alphanumeric, hyphens, dots, underscores, forward slashes, @, plus signs.
/// Rejects: semicolons, backticks, pipes, ampersands, dollar signs, etc.
pub fn validate_package_name(name: &str) -> Result<(), PkgError> {
    if name.is_empty() {
        return Ok(()); // empty name is valid (used for "upgrade all")
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

/// Run a package manager command and capture output.
pub async fn run_pkg_command(
    command: &str,
    args: &[&str],
    action: &str,
    package: &str,
) -> Result<PackageResult, PkgError> {
    validate_package_name(package)?;

    let output = tokio::process::Command::new(command)
        .args(args)
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
}
