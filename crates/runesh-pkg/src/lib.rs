#![deny(unsafe_code)]
//! Cross-platform package manager abstraction.
//!
//! Provides a uniform trait over apt, dnf, pacman, apk, zypper, brew,
//! winget, and FreeBSD pkg. Auto-detects the system package manager.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod apk;
pub mod apt;
pub mod brew;
pub mod detect;
pub mod dnf;
pub mod freebsd;
pub mod pacman;
pub mod runner;
pub mod winget;
pub mod zypper;

pub use apk::ApkManager;
pub use apt::AptManager;
pub use brew::BrewManager;
pub use dnf::DnfManager;
pub use freebsd::PkgManager;
pub use pacman::PacmanManager;
pub use winget::WingetManager;
pub use zypper::ZypperManager;

/// Information about an installed or available package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub update_available: Option<String>,
}

/// Result of a package operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageResult {
    pub success: bool,
    pub package: String,
    pub action: String,
    pub output: String,
    pub exit_code: i32,
}

/// Uniform package manager interface.
#[async_trait]
pub trait PackageManager: Send + Sync {
    /// Manager name (apt, dnf, pacman, etc.).
    fn name(&self) -> &str;

    /// List installed packages.
    async fn list_installed(&self) -> Result<Vec<PackageInfo>, PkgError>;

    /// Search for available packages.
    async fn search(&self, query: &str) -> Result<Vec<PackageInfo>, PkgError>;

    /// Install a package.
    async fn install(&self, package: &str) -> Result<PackageResult, PkgError>;

    /// Remove a package.
    async fn remove(&self, package: &str) -> Result<PackageResult, PkgError>;

    /// Upgrade a specific package. `package` must be non-empty.
    /// For upgrading every installed package, use [`PackageManager::upgrade_all`].
    async fn upgrade(&self, package: &str) -> Result<PackageResult, PkgError>;

    /// Upgrade every installed package.
    ///
    /// Default implementation delegates to `upgrade("")` so existing backends
    /// that use the "empty name means all" convention continue to work.
    async fn upgrade_all(&self) -> Result<PackageResult, PkgError> {
        self.upgrade("").await
    }

    /// Check for available updates.
    async fn available_updates(&self) -> Result<Vec<PackageInfo>, PkgError>;
}

/// Create the appropriate PackageManager for this system.
pub fn system_package_manager() -> Option<Box<dyn PackageManager>> {
    let pm_type = detect::detect()?;
    match pm_type {
        detect::PkgManagerType::Apt => Some(Box::new(AptManager)),
        detect::PkgManagerType::Dnf | detect::PkgManagerType::Yum => Some(Box::new(DnfManager)),
        detect::PkgManagerType::Pacman => Some(Box::new(PacmanManager)),
        detect::PkgManagerType::Apk => Some(Box::new(ApkManager)),
        detect::PkgManagerType::Zypper => Some(Box::new(ZypperManager)),
        detect::PkgManagerType::Brew => Some(Box::new(BrewManager)),
        detect::PkgManagerType::Winget => Some(Box::new(WingetManager)),
        detect::PkgManagerType::Pkg => Some(Box::new(PkgManager)),
        _ => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PkgError {
    #[error("package manager not found: {0}")]
    NotFound(String),
    #[error("command failed: {0}")]
    CommandFailed(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
