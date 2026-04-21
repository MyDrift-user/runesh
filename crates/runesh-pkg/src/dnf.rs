//! DNF/YUM package manager implementation (Fedora, RHEL, CentOS).

use async_trait::async_trait;

use crate::runner::{require_package_name, run_pkg_command};
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct DnfManager;

#[async_trait]
impl PackageManager for DnfManager {
    fn name(&self) -> &str {
        "dnf"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "rpm",
            &["-qa", "--queryformat", "%{NAME}\t%{VERSION}-%{RELEASE}\n"],
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
                let (name, version) = line.split_once('\t')?;
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
        let result = run_pkg_command("dnf", &["search", "--quiet", query], "search", query).await?;
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let (name, desc) = line.split_once(" : ")?;
                Some(PackageInfo {
                    name: name.split('.').next().unwrap_or(name).to_string(),
                    version: String::new(),
                    description: Some(desc.trim().to_string()),
                    installed: false,
                    update_available: None,
                })
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("dnf", &["install", "-y", package], "install", package).await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("dnf", &["remove", "-y", package], "remove", package).await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command("dnf", &["upgrade", "-y", package], "upgrade", package).await
    }

    async fn upgrade_all(&self) -> Result<PackageResult, PkgError> {
        run_pkg_command("dnf", &["upgrade", "-y"], "upgrade", "all").await
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result =
            run_pkg_command("dnf", &["check-update", "--quiet"], "check-updates", "").await?;
        // dnf check-update exits with 100 when updates are available
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && !line.is_empty() && !line.starts_with("Last metadata") {
                    Some(PackageInfo {
                        name: parts[0].split('.').next().unwrap_or(parts[0]).to_string(),
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
