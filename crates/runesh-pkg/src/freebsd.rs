//! FreeBSD pkg package manager implementation.

use async_trait::async_trait;

use crate::runner::{require_package_name, run_pkg_command};
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct PkgManager;

#[async_trait]
impl PackageManager for PkgManager {
    fn name(&self) -> &str {
        "pkg"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("pkg", &["query", "--all", "%n\t%v\t%c"], "list", "").await?;
        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() >= 2 {
                    Some(PackageInfo {
                        name: parts[0].to_string(),
                        version: parts[1].to_string(),
                        description: parts.get(2).map(|s| s.to_string()),
                        installed: true,
                        update_available: None,
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("pkg", &["search", query], "search", query).await?;
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                // Format: name-version                Comment
                let (name_ver, desc) = if let Some(pos) = line.find(char::is_whitespace) {
                    (&line[..pos], Some(line[pos..].trim()))
                } else {
                    (line, None)
                };
                // Split name from version at last hyphen
                let split = name_ver.rfind('-')?;
                let name = &name_ver[..split];
                let version = &name_ver[split + 1..];
                Some(PackageInfo {
                    name: name.to_string(),
                    version: version.to_string(),
                    description: desc.map(|s| s.to_string()),
                    installed: false,
                    update_available: None,
                })
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("pkg", &["install", "-y", package], "install", package).await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("pkg", &["delete", "-y", package], "remove", package).await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("pkg", &["install", "-y", package], "upgrade", package).await
    }

    async fn upgrade_all(&self) -> Result<PackageResult, PkgError> {
        run_pkg_command("pkg", &["upgrade", "-y"], "upgrade", "all").await
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("pkg", &["version", "-vRL="], "check-updates", "").await?;
        Ok(result
            .output
            .lines()
            .filter(|l| l.contains('<'))
            .filter_map(|line| {
                // Format: name-version                    <   needs updating (remote has version)
                let name_ver = line.split_whitespace().next()?;
                let split = name_ver.rfind('-')?;
                let name = &name_ver[..split];
                let new_ver = line
                    .split("has ")
                    .nth(1)
                    .map(|s| s.trim_end_matches(')').to_string());
                Some(PackageInfo {
                    name: name.to_string(),
                    version: name_ver[split + 1..].to_string(),
                    description: None,
                    installed: true,
                    update_available: new_ver,
                })
            })
            .collect())
    }
}
