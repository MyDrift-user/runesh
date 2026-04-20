//! APK package manager implementation (Alpine Linux).

use async_trait::async_trait;

use crate::runner::run_pkg_command;
use crate::{PackageInfo, PackageManager, PackageResult, PkgError};

pub struct ApkManager;

#[async_trait]
impl PackageManager for ApkManager {
    fn name(&self) -> &str {
        "apk"
    }

    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("apk", &["list", "--installed"], "list", "").await?;
        if !result.success {
            return Err(PkgError::CommandFailed(result.output));
        }
        Ok(result
            .output
            .lines()
            .filter_map(|line| {
                // Format: name-version-rrelease arch {origin} (license) [installed]
                let bracket = line.find(" {")?;
                let name_ver = &line[..bracket];
                // Split name from version at last hyphen before a digit
                let mut split_pos = None;
                for (i, c) in name_ver.char_indices().rev() {
                    if c == '-' {
                        if name_ver[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                            split_pos = Some(i);
                            break;
                        }
                    }
                }
                let (name, version) = match split_pos {
                    Some(pos) => (&name_ver[..pos], &name_ver[pos + 1..]),
                    None => (name_ver, ""),
                };
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
        let result =
            run_pkg_command("apk", &["search", "--description", query], "search", query).await?;
        Ok(result
            .output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                // Format: name-version
                let mut split_pos = None;
                for (i, c) in line.char_indices().rev() {
                    if c == '-' {
                        if line[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                            split_pos = Some(i);
                            break;
                        }
                    }
                }
                let (name, version) = match split_pos {
                    Some(pos) => (&line[..pos], &line[pos + 1..]),
                    None => (line, ""),
                };
                PackageInfo {
                    name: name.to_string(),
                    version: version.to_string(),
                    description: None,
                    installed: false,
                    update_available: None,
                }
            })
            .collect())
    }

    async fn install(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command(
            "apk",
            &["add", "--no-interactive", package],
            "install",
            package,
        )
        .await
    }

    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError> {
        run_pkg_command(
            "apk",
            &["del", "--no-interactive", package],
            "remove",
            package,
        )
        .await
    }

    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError> {
        if package.is_empty() {
            run_pkg_command("apk", &["upgrade", "--no-interactive"], "upgrade", "all").await
        } else {
            run_pkg_command(
                "apk",
                &["add", "--no-interactive", "--upgrade", package],
                "upgrade",
                package,
            )
            .await
        }
    }

    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError> {
        let result = run_pkg_command("apk", &["list", "--upgradable"], "check-updates", "").await?;
        Ok(result
            .output
            .lines()
            .filter(|l| !l.is_empty() && l.contains("upgradable"))
            .filter_map(|line| {
                let bracket = line.find(" {")?;
                let name_ver = &line[..bracket];
                let mut split_pos = None;
                for (i, c) in name_ver.char_indices().rev() {
                    if c == '-' && name_ver[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                        split_pos = Some(i);
                        break;
                    }
                }
                let (name, version) = match split_pos {
                    Some(pos) => (&name_ver[..pos], &name_ver[pos + 1..]),
                    None => (name_ver, ""),
                };
                Some(PackageInfo {
                    name: name.to_string(),
                    version: String::new(),
                    description: None,
                    installed: true,
                    update_available: Some(version.to_string()),
                })
            })
            .collect())
    }
}
