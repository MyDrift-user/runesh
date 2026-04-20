//! Pacman package manager implementation (Arch Linux, Manjaro).

use async_trait::async_trait;

use crate::runner::run_pkg_command;
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct PacmanManager;

#[async_trait]
impl PackageManager for PacmanManager {
    fn name(&self) -> &str {
        "pacman"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("pacman", &["-Q"], "list", "").await?;
        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                let version = parts.next().unwrap_or("");
                Some(PackageInfo {
                    name: name.to_string(),
                    version: version.to_string(),
                    description: None,
                    installed: true,
                    update_available: None,
                })
            })
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("pacman", &["-Ss", query], "search", query).await?;
        let mut packages = Vec::new();
        let mut lines = result.output.lines().peekable();
        while let Some(line) = lines.next() {
            // Format: repo/name version
            //     Description
            if line.starts_with(' ') {
                continue;
            }
            if let Some((repo_name, version)) = line.split_once(' ') {
                let name = repo_name.split('/').last().unwrap_or(repo_name).to_string();
                let desc = lines
                    .peek()
                    .filter(|l| l.starts_with(' '))
                    .map(|l| l.trim().to_string());
                packages.push(PackageInfo {
                    name,
                    version: version.trim().to_string(),
                    description: desc,
                    installed: false,
                    update_available: None,
                });
            }
        }
        Ok(packages)
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command(
            "pacman",
            &["-S", "--noconfirm", package],
            "install",
            package,
        )
        .await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command("pacman", &["-R", "--noconfirm", package], "remove", package).await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        if package.is_empty() {
            run_pkg_command("pacman", &["-Syu", "--noconfirm"], "upgrade", "all").await
        } else {
            run_pkg_command(
                "pacman",
                &["-S", "--noconfirm", package],
                "upgrade",
                package,
            )
            .await
        }
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        // checkupdates is from pacman-contrib, safer than pacman -Qu
        let result = run_pkg_command("checkupdates", &[], "check-updates", "").await;
        let output = match result {
            Ok(r) => r.output,
            Err(_) => {
                // Fallback to pacman -Qu
                let r = run_pkg_command("pacman", &["-Qu"], "check-updates", "").await?;
                r.output
            }
        };
        Ok(output
            .lines()
            .filter_map(|line| {
                // Format: name old_version -> new_version
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 && parts[2] == "->" {
                    Some(PackageInfo {
                        name: parts[0].to_string(),
                        version: parts[1].to_string(),
                        description: None,
                        installed: true,
                        update_available: Some(parts[3].to_string()),
                    })
                } else if parts.len() >= 2 {
                    // pacman -Qu format: name new_version
                    Some(PackageInfo {
                        name: parts[0].to_string(),
                        version: String::new(),
                        description: None,
                        installed: true,
                        update_available: Some(parts[1].to_string()),
                    })
                } else {
                    None
                }
            })
            .collect())
    }
}
