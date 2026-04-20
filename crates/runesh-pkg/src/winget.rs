//! WinGet package manager implementation (Windows).
//!
//! Uses the `winget` CLI for package operations. Parses the structured
//! output to extract package information.

use async_trait::async_trait;

use crate::runner::run_pkg_command;
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct WingetManager;

#[async_trait]
impl PackageManager for WingetManager {
    fn name(&self) -> &str {
        "winget"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "winget",
            &[
                "list",
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "list",
            "",
        )
        .await?;

        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }

        Ok(parse_winget_table(&result.output, true))
    }

    async fn search(&self, query: &str) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "winget",
            &[
                "search",
                query,
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "search",
            query,
        )
        .await?;

        Ok(parse_winget_table(&result.output, false))
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command(
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
        )
        .await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command(
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
        )
        .await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        if package.is_empty() {
            run_pkg_command(
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
            )
            .await
        } else {
            run_pkg_command(
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
            )
            .await
        }
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "winget",
            &[
                "upgrade",
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
            "check-updates",
            "",
        )
        .await?;

        Ok(parse_winget_upgrade_table(&result.output))
    }
}

/// Parse winget's table output into PackageInfo entries.
///
/// Winget outputs tables with headers like:
///   Name           Id                  Version   Available Source
///   -----------------------------------------------------------
///   Firefox        Mozilla.Firefox     125.0     126.0     winget
fn parse_winget_table(output: &str, installed: bool) -> Vec<PackageInfo> {
    let lines: Vec<&str> = output.lines().collect();

    // Find the separator line (all dashes)
    let sep_idx = lines
        .iter()
        .position(|l| l.chars().all(|c| c == '-' || c == ' ') && l.contains("---"));

    let header_idx = match sep_idx {
        Some(idx) if idx > 0 => idx - 1,
        _ => return vec![],
    };

    let header = lines[header_idx];
    let data_start = sep_idx.unwrap() + 1;

    // Detect column positions from header
    let name_start = 0;
    let id_start = header.find("Id").unwrap_or(0);
    let version_start = header.find("Version").unwrap_or(0);

    if id_start == 0 || version_start == 0 {
        return vec![];
    }

    lines[data_start..]
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
        .collect()
}

/// Parse winget upgrade output which has an "Available" column.
fn parse_winget_upgrade_table(output: &str) -> Vec<PackageInfo> {
    let lines: Vec<&str> = output.lines().collect();

    let sep_idx = lines
        .iter()
        .position(|l| l.chars().all(|c| c == '-' || c == ' ') && l.contains("---"));

    let header_idx = match sep_idx {
        Some(idx) if idx > 0 => idx - 1,
        _ => return vec![],
    };

    let header = lines[header_idx];
    let data_start = sep_idx.unwrap() + 1;

    let id_start = header.find("Id").unwrap_or(0);
    let version_start = header.find("Version").unwrap_or(0);
    let available_start = header.find("Available").unwrap_or(0);

    if id_start == 0 || version_start == 0 || available_start == 0 {
        return vec![];
    }

    lines[data_start..]
        .iter()
        .filter(|l| l.len() > available_start && !l.trim().is_empty())
        .filter_map(|line| {
            // Skip summary lines like "X upgrades available."
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
        .collect()
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
        let packages = parse_winget_table(output, true);
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
        let packages = parse_winget_upgrade_table(output);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].update_available, Some("126.0".into()));
    }

    #[test]
    fn parse_empty_output() {
        let packages = parse_winget_table("No installed package found.\n", true);
        assert!(packages.is_empty());
    }
}
