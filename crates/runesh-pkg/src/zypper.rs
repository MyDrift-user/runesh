//! Zypper package manager implementation (openSUSE, SLES).

use async_trait::async_trait;

use crate::runner::{require_package_name, run_pkg_command};
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct ZypperManager;

#[async_trait]
impl PackageManager for ZypperManager {
    fn name(&self) -> &str {
        "zypper"
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
        let result = run_pkg_command(
            "zypper",
            &["--non-interactive", "search", query],
            "search",
            query,
        )
        .await?;
        Ok(result
            .output
            .lines()
            .filter(|l| l.contains('|') && !l.starts_with("--") && !l.starts_with("S "))
            .filter_map(|line| {
                let cols: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                if cols.len() >= 3 {
                    Some(PackageInfo {
                        name: cols[1].to_string(),
                        version: if cols.len() >= 4 {
                            cols[3].to_string()
                        } else {
                            String::new()
                        },
                        description: cols.get(2).map(|s| s.to_string()),
                        installed: cols[0].contains('i'),
                        update_available: None,
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command(
            "zypper",
            &["--non-interactive", "install", package],
            "install",
            package,
        )
        .await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command(
            "zypper",
            &["--non-interactive", "remove", package],
            "remove",
            package,
        )
        .await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        require_package_name(package)?;
        run_pkg_command(
            "zypper",
            &["--non-interactive", "update", package],
            "upgrade",
            package,
        )
        .await
    }

    async fn upgrade_all(&self) -> Result<PackageResult, PkgError> {
        run_pkg_command("zypper", &["--non-interactive", "update"], "upgrade", "all").await
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command(
            "zypper",
            &["--non-interactive", "list-updates"],
            "check-updates",
            "",
        )
        .await?;
        Ok(result
            .output
            .lines()
            .filter(|l| l.contains('|') && !l.starts_with("--") && !l.starts_with("S "))
            .filter_map(|line| {
                let cols: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                if cols.len() >= 5 {
                    Some(PackageInfo {
                        name: cols[2].to_string(),
                        version: cols[3].to_string(),
                        description: None,
                        installed: true,
                        update_available: Some(cols[4].to_string()),
                    })
                } else {
                    None
                }
            })
            .collect())
    }
}
