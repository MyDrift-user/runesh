//! WinGet package manager implementation (Windows).
//!
//! Uses the `winget` CLI for package operations. Parses the structured
//! output to extract package information.

use async_trait::async_trait;

use crate::runner::{require_package_name, run_pkg_command_env};
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

/// Environment overrides applied to every winget invocation so the output is
/// in English (our parser assumes `Name` / `Id` column headers) and free of
/// locale-driven column shifts.
fn winget_env() -> Vec<(&'static str, &'static str)> {
    vec![("DOTNET_CLI_UI_LANGUAGE", "en-US"), ("LC_ALL", "C")]
}

pub struct WingetManager;

#[async_trait]
impl PackageManager for WingetManager {
    fn name(&self) -> &str {
        "winget"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let env = winget_env();
        let result = run_pkg_command_env(
            "winget",
            &[
                "list",
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "list",
            "",
            &env,
        )
        .await?;

        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }

        parse_winget_table(&result.output, true)
    }

    async fn search(&self, query: &str) -> Result<Vec<PackageInfo>, PkgError> {
        let env = winget_env();
        let result = run_pkg_command_env(
            "winget",
            &[
                "search",
                query,
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "search",
            query,
            &env,
        )
        .await?;

        parse_winget_table(&result.output, false)
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        let env = winget_env();
        run_pkg_command_env(
            "winget",
            &[
                "install",
                "--id",
                package,
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
            "install",
            package,
            &env,
        )
        .await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        let env = winget_env();
        run_pkg_command_env(
            "winget",
            &[
                "uninstall",
                "--id",
                package,
                "--silent",
                "--disable-interactivity",
            ],
            "remove",
            package,
            &env,
        )
        .await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        let env = winget_env();
        run_pkg_command_env(
            "winget",
            &[
                "upgrade",
                "--id",
                package,
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
            "upgrade",
            package,
            &env,
        )
        .await
    }

    async fn upgrade_all(&self) -> Result<PackageResult, PkgError> {
        let env = winget_env();
        run_pkg_command_env(
            "winget",
            &[
                "upgrade",
                "--all",
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
            "upgrade",
            "all",
            &env,
        )
        .await
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let env = winget_env();
        let result = run_pkg_command_env(
            "winget",
            &[
                "upgrade",
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "check-updates",
            "",
            &env,
        )
        .await?;

        parse_winget_upgrade_table(&result.output)
    }
}

/// Strip a UTF-8 BOM and any UTF-16 BOM remnants from winget's output.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Parse winget's table output into PackageInfo entries.
///
/// Winget outputs tables with headers like:
///   Name           Id                  Version   Available Source
///   -----------------------------------------------------------
///   Firefox        Mozilla.Firefox     125.0     126.0     winget
///
/// Returns `Err(ParseError)` when the header is missing the required
/// `Name` / `Id` columns (for example, when the output is a localized
/// error string and we have no anchor to parse columns from).
fn parse_winget_table(output: &str, installed: bool) -> Result<Vec<PackageInfo>, PkgError> {
    let output = strip_bom(output);
    let lines: Vec<&str> = output.lines().collect();

    let sep_idx = lines
        .iter()
        .position(|l| l.chars().all(|c| c == '-' || c == ' ') && l.contains("---"));

    let header_idx = match sep_idx {
        Some(idx) if idx > 0 => idx - 1,
        _ => return Ok(vec![]),
    };

    let header = strip_bom(lines[header_idx]);
    let data_start = sep_idx.unwrap() + 1;

    let name_start = 0;
    let id_start = header
        .find("Id")
        .ok_or_else(|| PkgError::ParseError("winget header missing 'Id' column".into()))?;
    let version_start = header
        .find("Version")
        .ok_or_else(|| PkgError::ParseError("winget header missing 'Version' column".into()))?;
    if !header.contains("Name") {
        return Err(PkgError::ParseError(
            "winget header missing 'Name' column".into(),
        ));
    }

    Ok(lines[data_start..]
        .iter()
        .filter(|l| l.len() > version_start && !l.trim().is_empty())
        .filter_map(|line| {
            let name = line.get(name_start..id_start)?.trim().to_string();
            let id = line.get(id_start..version_start)?.trim().to_string();
            let rest = line.get(version_start..)?.trim();
            let version = rest.split_whitespace().next().unwrap_or("").to_string();

            if id.is_empty() {
                return None;
            }

            Some(PackageInfo {
                name: if name.is_empty() { id.clone() } else { name },
                version,
                description: None,
                installed,
                update_available: None,
            })
        })
        .collect())
}

/// Parse winget upgrade output which has an "Available" column.
fn parse_winget_upgrade_table(output: &str) -> Result<Vec<PackageInfo>, PkgError> {
    let output = strip_bom(output);
    let lines: Vec<&str> = output.lines().collect();

    let sep_idx = lines
        .iter()
        .position(|l| l.chars().all(|c| c == '-' || c == ' ') && l.contains("---"));

    let header_idx = match sep_idx {
        Some(idx) if idx > 0 => idx - 1,
        _ => return Ok(vec![]),
    };

    let header = strip_bom(lines[header_idx]);
    let data_start = sep_idx.unwrap() + 1;

    let id_start = header
        .find("Id")
        .ok_or_else(|| PkgError::ParseError("winget upgrade header missing 'Id'".into()))?;
    let version_start = header
        .find("Version")
        .ok_or_else(|| PkgError::ParseError("winget upgrade header missing 'Version'".into()))?;
    let available_start = header
        .find("Available")
        .ok_or_else(|| PkgError::ParseError("winget upgrade header missing 'Available'".into()))?;

    Ok(lines[data_start..]
        .iter()
        .filter(|l| l.len() > available_start && !l.trim().is_empty())
        .filter_map(|line| {
            if line.contains("upgrades available") {
                return None;
            }

            let id = line.get(id_start..version_start)?.trim().to_string();
            let version = line.get(version_start..available_start)?.trim().to_string();
            let available = line.get(available_start..)?.trim();
            let new_version = available
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();

            if id.is_empty() || new_version.is_empty() {
                return None;
            }

            Some(PackageInfo {
                name: id.clone(),
                version,
                description: None,
                installed: true,
                update_available: Some(new_version),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_winget_list_output() {
        let output = r#"Name             Id                      Version
-----------------------------------------------------------
Firefox          Mozilla.Firefox         125.0.2
Visual Studio    Microsoft.VisualStudio  17.9.6
"#;
        let packages = parse_winget_table(output, true).unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "Firefox");
        assert_eq!(packages[0].version, "125.0.2");
        assert!(packages[0].installed);
    }

    #[test]
    fn parse_winget_upgrade_output() {
        let output = r#"Name             Id                      Version   Available Source
---------------------------------------------------------------------------
Firefox          Mozilla.Firefox         125.0.2   126.0     winget
Git              Git.Git                 2.44.0    2.45.0    winget
2 upgrades available.
"#;
        let packages = parse_winget_upgrade_table(output).unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].update_available, Some("126.0".into()));
    }

    #[test]
    fn parse_empty_output() {
        let packages = parse_winget_table("No installed package found.\n", true).unwrap();
        assert!(packages.is_empty());
    }

    #[test]
    fn parse_strips_utf8_bom() {
        let output = "\u{feff}Name             Id                      Version\n-----------------------------------------------------------\nFirefox          Mozilla.Firefox         125.0.2\n";
        let packages = parse_winget_table(output, true).unwrap();
        assert_eq!(packages.len(), 1);
    }

    #[test]
    fn parse_errors_on_missing_name_column() {
        // No Name header - parser must fail explicitly.
        let output = "Id                      Version\n--------------------------------\nMozilla.Firefox         125.0.2\n";
        let err = parse_winget_table(output, true).unwrap_err();
        match err {
            PkgError::ParseError(msg) => assert!(msg.contains("Name")),
            other => panic!("expected ParseError, got {other:?}"),
        }
    }
}
