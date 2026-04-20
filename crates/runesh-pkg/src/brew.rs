//! Homebrew package manager implementation (macOS).

use async_trait::async_trait;

use crate::runner::run_pkg_command;
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct BrewManager;

#[async_trait]
impl PackageManager for BrewManager {
    fn name(&self) -> &str {
        "brew"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("brew", &["list", "--versions"], "list", "").await?;
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
        let result = run_pkg_command("brew", &["search", query], "search", query).await?;
        Ok(result
            .output
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with("==>"))
            .map(|name| PackageInfo {
                name: name.trim().to_string(),
                version: String::new(),
                description: None,
                installed: false,
                update_available: None,
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command("brew", &["install", package], "install", package).await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command("brew", &["uninstall", package], "remove", package).await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        if package.is_empty() {
            run_pkg_command("brew", &["upgrade"], "upgrade", "all").await
        } else {
            run_pkg_command("brew", &["upgrade", package], "upgrade", package).await
        }
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result =
            run_pkg_command("brew", &["outdated", "--verbose"], "check-updates", "").await?;
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                // Format: "package (installed) < available"
                let name = line.split_whitespace().next()?;
                let new_ver = line.split("< ").nth(1).map(|s| s.trim().to_string());
                Some(PackageInfo {
                    name: name.to_string(),
                    version: String::new(),
                    description: None,
                    installed: true,
                    update_available: new_ver,
                })
            })
            .collect())
    }
}
