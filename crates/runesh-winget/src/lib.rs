#![deny(unsafe_code)]
//! WinGet REST source server.
//!
//! Implements the Microsoft.Rest protocol so `winget` clients can
//! search, browse, and install packages from your private repository.
//!
//! Register with: `winget source add --name MyRepo --arg https://host/api --type Microsoft.Rest`

pub mod auth;
pub mod manifest;
pub mod repo;

#[cfg(feature = "axum")]
pub mod handlers;

pub use auth::{AdminAuth, AuthError, StaticTokenAuth, admin_token_from_env};
pub use manifest::{
    Architecture, DefaultLocale, Installer, InstallerSwitches, InstallerType, ManifestError,
    ManifestPolicy, Package, PackageVersion, Scope, SearchRequest, validate_installer_url,
    validate_sha256,
};
pub use repo::WingetRepo;

#[cfg(feature = "axum")]
pub use handlers::{WingetState, winget_router};
