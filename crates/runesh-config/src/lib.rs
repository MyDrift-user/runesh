//! Desired system state for a host, and a reconciler that applies it.
//!
//! `runesh-config` complements [`runesh-baseline`] (which audits drift) with
//! the push-configure half: the server ships a [`ConfigSpec`] to the agent,
//! and the agent applies each section through the platform-native APIs.
//!
//! The crate deliberately does not shell out to PowerShell, `bash -c`, or
//! other interpreters. Each subsystem has a trait with platform-specific
//! implementations that call native APIs (Win32 + WMI on Windows,
//! systemd-dbus / NetworkManager dbus / writable files under `/etc` on Linux,
//! `scutil` / `dscl` / `defaults` / the `SystemConfiguration` framework on
//! macOS). This keeps the attack surface small, the error handling typed,
//! and the applier usable from a locked-down agent context where
//! interpreters may not be present.
//!
//! The v1 applier fully implements hostname management across Windows,
//! Linux, and macOS. The other subsystems (timezone, users, network,
//! display, desktop) are declared in [`ConfigSpec`] and surfaced through
//! typed traits, but their `apply` paths return
//! [`ConfigError::NotSupported`] with a clear next-step message. That makes
//! the extension path obvious without leaving half-built code paths live.
//!
//! # Example
//!
//! ```no_run
//! use runesh_config::{ConfigSpec, HostnameConfig, Reconciler};
//!
//! # async fn _demo() -> Result<(), runesh_config::ConfigError> {
//! let spec = ConfigSpec {
//!     hostname: Some(HostnameConfig { name: "edge-01".into() }),
//!     ..ConfigSpec::default()
//! };
//!
//! let reconciler = Reconciler::for_current_os();
//! let report = reconciler.apply(&spec).await?;
//! assert!(report.changed_sections().contains(&"hostname"));
//! # Ok(()) }
//! ```
//!
//! This crate uses `unsafe` only to call the handful of Win32 FFI functions
//! that have no stable-crate wrapper (`SetComputerNameExW`, display config
//! APIs, user management via netapi32). Every unsafe block is documented at
//! the call site with the invariants the caller relies on.

use serde::{Deserialize, Serialize};

pub mod error;
pub mod platform;
pub mod spec;

pub use error::ConfigError;
pub use platform::{ConfigApplier, Reconciler};
pub use spec::{
    ConfigSpec, DesktopConfig, DisplayConfig, DnsConfig, HostnameConfig, IpAddressing,
    MonitorConfig, NetworkAdapterConfig, NetworkConfig, Position, Resolution, ShellConfig,
    TaskbarConfig, TaskbarPosition, ThemeConfig, TimezoneConfig, UserConfig, UsersConfig,
    WallpaperConfig, WallpaperFit,
};

/// Result of applying (or dry-running) a [`ConfigSpec`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    /// Per-section outcomes.
    pub sections: Vec<SectionReport>,
}

impl ApplyReport {
    /// Names of sections that actually changed.
    pub fn changed_sections(&self) -> Vec<&str> {
        self.sections
            .iter()
            .filter(|s| s.outcome == SectionOutcome::Changed)
            .map(|s| s.name.as_str())
            .collect()
    }

    /// Names of sections that could not be applied because the subsystem is
    /// not implemented on the current platform.
    pub fn unsupported_sections(&self) -> Vec<&str> {
        self.sections
            .iter()
            .filter(|s| matches!(s.outcome, SectionOutcome::NotSupported))
            .map(|s| s.name.as_str())
            .collect()
    }

    pub(crate) fn push(&mut self, name: impl Into<String>, outcome: SectionOutcome) {
        self.sections.push(SectionReport {
            name: name.into(),
            outcome,
            detail: None,
        });
    }

    pub(crate) fn push_with_detail(
        &mut self,
        name: impl Into<String>,
        outcome: SectionOutcome,
        detail: impl Into<String>,
    ) {
        self.sections.push(SectionReport {
            name: name.into(),
            outcome,
            detail: Some(detail.into()),
        });
    }
}

/// One section's outcome after [`ConfigApplier::apply`] (or `dry_run`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionReport {
    pub name: String,
    pub outcome: SectionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Status of a single section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionOutcome {
    /// Subsystem was already in the desired state; nothing to do.
    AlreadyCurrent,
    /// Subsystem was modified to match the spec.
    Changed,
    /// Subsystem would be modified (returned by `dry_run`).
    WouldChange,
    /// Not included in the spec.
    Skipped,
    /// Applier is not implemented on this platform or subsystem.
    NotSupported,
    /// Applier ran and failed.
    Failed,
}
