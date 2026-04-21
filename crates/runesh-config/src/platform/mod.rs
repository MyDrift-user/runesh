//! Platform dispatch for [`ConfigApplier`]. Selects the correct backend
//! (`windows`, `linux`, `macos`) at compile time based on `target_os`.

use async_trait::async_trait;

use crate::{ApplyReport, ConfigError, ConfigSpec, SectionOutcome};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

// ── Public applier trait ────────────────────────────────────────────────────

/// Applies a [`ConfigSpec`] to the current machine.
#[async_trait]
pub trait ConfigApplier: Send + Sync {
    /// Enforce the spec. Sections missing from the spec are left untouched;
    /// each included section is reconciled with the observed state.
    async fn apply(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError>;

    /// Same as [`apply`](Self::apply) but with no side effects; the report
    /// records `WouldChange` for sections that would change.
    async fn dry_run(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError>;
}

// ── Reconciler: the actual public [`ConfigApplier`] ─────────────────────────

/// Selects and dispatches to the per-OS backend. One reconciler is used per
/// agent; it owns the platform-specific backend.
pub struct Reconciler {
    inner: Box<dyn ConfigApplier>,
}

impl Reconciler {
    /// Build a reconciler for the OS this binary is running on.
    pub fn for_current_os() -> Self {
        #[cfg(target_os = "windows")]
        let inner: Box<dyn ConfigApplier> = Box::new(windows::WindowsApplier::new());
        #[cfg(target_os = "linux")]
        let inner: Box<dyn ConfigApplier> = Box::new(linux::LinuxApplier::new());
        #[cfg(target_os = "macos")]
        let inner: Box<dyn ConfigApplier> = Box::new(macos::MacOsApplier::new());
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        let inner: Box<dyn ConfigApplier> = Box::new(UnsupportedOs);

        Self { inner }
    }

    pub async fn apply(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        self.inner.apply(spec).await
    }

    pub async fn dry_run(&self, spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        self.inner.dry_run(spec).await
    }
}

/// Fallback for targets without a dedicated backend.
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
struct UnsupportedOs;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
#[async_trait]
impl ConfigApplier for UnsupportedOs {
    async fn apply(&self, _spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        Err(ConfigError::NotSupported(format!(
            "no backend for target_os = {}",
            std::env::consts::OS
        )))
    }

    async fn dry_run(&self, _spec: &ConfigSpec) -> Result<ApplyReport, ConfigError> {
        self.apply(_spec).await
    }
}

// ── Shared per-section runner used by every platform backend ────────────────

pub(crate) async fn run_spec<B>(
    backend: &B,
    spec: &ConfigSpec,
    dry: bool,
) -> Result<ApplyReport, ConfigError>
where
    B: PlatformBackend + ?Sized,
{
    let mut report = ApplyReport::default();

    dispatch(
        &mut report,
        "hostname",
        spec.hostname.as_ref(),
        dry,
        |c, d| backend.hostname(c, d),
    )
    .await;

    dispatch(
        &mut report,
        "timezone",
        spec.timezone.as_ref(),
        dry,
        |c, d| backend.timezone(c, d),
    )
    .await;

    dispatch(&mut report, "users", spec.users.as_ref(), dry, |c, d| {
        backend.users(c, d)
    })
    .await;

    dispatch(
        &mut report,
        "network",
        spec.network.as_ref(),
        dry,
        |c, d| backend.network(c, d),
    )
    .await;

    dispatch(
        &mut report,
        "display",
        spec.display.as_ref(),
        dry,
        |c, d| backend.display(c, d),
    )
    .await;

    dispatch(
        &mut report,
        "desktop",
        spec.desktop.as_ref().filter(|m| !m.is_empty()),
        dry,
        |c, d| backend.desktop(c, d),
    )
    .await;

    dispatch(&mut report, "shell", spec.shell.as_ref(), dry, |c, d| {
        backend.shell(c, d)
    })
    .await;

    Ok(report)
}

async fn dispatch<'a, C, F, Fut>(
    report: &mut ApplyReport,
    name: &str,
    cfg: Option<&'a C>,
    dry: bool,
    f: F,
) where
    F: FnOnce(&'a C, bool) -> Fut,
    Fut: std::future::Future<Output = Result<SectionOutcome, ConfigError>>,
{
    let Some(cfg) = cfg else {
        report.push(name, SectionOutcome::Skipped);
        return;
    };
    match f(cfg, dry).await {
        Ok(outcome) => report.push(name, outcome),
        Err(ConfigError::NotSupported(msg)) => {
            report.push_with_detail(name, SectionOutcome::NotSupported, msg)
        }
        Err(e) => report.push_with_detail(name, SectionOutcome::Failed, e.to_string()),
    }
}

/// Per-platform backend. Each method returns an outcome for the section or
/// an error. All methods accept a `dry` flag; when true the backend must not
/// mutate state and must report `WouldChange` if a change is needed.
#[async_trait]
pub(crate) trait PlatformBackend: Send + Sync {
    async fn hostname(
        &self,
        cfg: &crate::HostnameConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn timezone(
        &self,
        cfg: &crate::TimezoneConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn users(
        &self,
        cfg: &crate::UsersConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn network(
        &self,
        cfg: &crate::NetworkConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn display(
        &self,
        cfg: &crate::DisplayConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn desktop(
        &self,
        cfg: &std::collections::BTreeMap<String, crate::DesktopConfig>,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;

    async fn shell(
        &self,
        cfg: &crate::ShellConfig,
        dry: bool,
    ) -> Result<SectionOutcome, ConfigError>;
}
