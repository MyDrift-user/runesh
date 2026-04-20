//! APT package manager implementation (Debian, Ubuntu).

use async_trait::async_trait;

use crate::runner::run_pkg_command;
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct AptManager;

#[async_trait]
impl PackageManager for AptManager {
    fn name(&self) -> &str {
        "apt"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "dpkg-query",
            &["-W", "-f=${Package}\t${Version}\t${Status}\n"],
            "list",
            "",
        )
        .await?;
        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 3 && parts[2].contains("installed") {
                    Some(PackageInfo {
                        name: parts[0].to_string(),
                        version: parts[1].to_string(),
                        description: None,
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
        let result = run_pkg_command(
            "apt-cache",
            &["search", "--names-only", query],
            "search",
            query,
        )
        .await?;
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let (name, desc) = line.split_once(" - ")?;
                Some(PackageInfo {
                    name: name.trim().to_string(),
                    version: String::new(),
                    description: Some(desc.trim().to_string()),
                    installed: false,
                    update_available: None,
                })
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command("apt-get", &["install", "-y", package], "install", package).await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command("apt-get", &["remove", "-y", package], "remove", package).await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        if package.is_empty() {
            run_pkg_command("apt-get", &["upgrade", "-y"], "upgrade", "all").await
        } else {
            run_pkg_command(
                "apt-get",
                &["install", "-y", "--only-upgrade", package],
                "upgrade",
                package,
            )
            .await
        }
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("apt-get", &["-s", "upgrade"], "check-updates", "").await?;
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                if line.starts_with("Inst ") {
                    let parts: Vec<&str> = line.splitn(4, ' ').collect();
                    if parts.len() >= 3 {
                        let name = parts[1].to_string();
                        let version = parts[2].trim_matches(&['[', ']'][..]).to_string();
                        return Some(PackageInfo {
                            name,
                            version: String::new(),
                            description: None,
                            installed: true,
                            update_available: Some(version),
                        });
                    }
                }
                None
            })
            .collect())
    }
}
