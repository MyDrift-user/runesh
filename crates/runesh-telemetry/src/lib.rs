//! # runesh-telemetry
//!
//! Sentry / GlitchTip error reporting for RUNESH services.
//!
//! GlitchTip is wire-compatible with the Sentry SDK protocol, so the official
//! `sentry` crate works against either backend with no code changes — just
//! point `RUNESH_SENTRY_DSN` at your GlitchTip instance.
//!
//! ## Usage (binary `main.rs`)
//!
//! ```no_run
//! # fn main() {
//!     let _guard = runesh_telemetry::init(runesh_telemetry::Config::from_env(
//!         env!("CARGO_PKG_NAME"),
//!         env!("CARGO_PKG_VERSION"),
//!     ));
//!
//!     // Wire the tracing layer into your subscriber so `tracing::error!`
//!     // calls become Sentry events automatically.
//!     #[cfg(feature = "tracing-layer")]
//!     {
//!         use tracing_subscriber::prelude::*;
//!         tracing_subscriber::registry()
//!             .with(tracing_subscriber::fmt::layer())
//!             .with(runesh_telemetry::tracing_layer())
//!             .init();
//!     }
//!
//!     // ... your service ...
//! }
//! ```
//!
//! ## Axum integration
//!
//! Enable the `axum` feature and add the middleware:
//!
//! ```ignore
//! let app = Router::new()
//!     .route("/", get(handler))
//!     .layer(runesh_telemetry::axum::layer());
//! ```
//!
//! ## Environment variables
//!
//! - `RUNESH_SENTRY_DSN` — DSN string. If unset, telemetry is disabled (no-op).
//! - `RUNESH_ENV`        — environment name (e.g. "production", "staging"). Default: "development".
//! - `RUNESH_SAMPLE_RATE` — performance traces sample rate, `0.0`..`1.0`. Default: `0.0` (errors only).
//! - `RUNESH_TELEMETRY_DEBUG` — set to `1` to log Sentry's own debug output.

use std::borrow::Cow;

pub use sentry::ClientInitGuard as TelemetryGuard;

/// Configuration for [`init`].
#[derive(Debug, Clone)]
pub struct Config {
    /// Sentry/GlitchTip DSN. If `None`, [`init`] is a no-op.
    pub dsn: Option<String>,
    /// Service name (typically `env!("CARGO_PKG_NAME")`).
    pub service: Cow<'static, str>,
    /// Service version (typically `env!("CARGO_PKG_VERSION")`).
    pub version: Cow<'static, str>,
    /// Deployment environment ("production", "staging", "development", ...).
    pub environment: Cow<'static, str>,
    /// Performance traces sample rate (`0.0`..`1.0`). `0.0` disables tracing entirely.
    pub traces_sample_rate: f32,
    /// Enable Sentry's internal debug logging.
    pub debug: bool,
}

impl Config {
    /// Build a config from `RUNESH_*` environment variables.
    ///
    /// `service` and `version` should typically come from `CARGO_PKG_NAME` /
    /// `CARGO_PKG_VERSION` so the release in Sentry matches the binary you shipped.
    pub fn from_env(
        service: impl Into<Cow<'static, str>>,
        version: impl Into<Cow<'static, str>>,
    ) -> Self {
        let service = service.into();
        let version = version.into();
        Self {
            dsn: std::env::var("RUNESH_SENTRY_DSN")
                .ok()
                .filter(|s| !s.is_empty()),
            service,
            version,
            environment: std::env::var("RUNESH_ENV")
                .ok()
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed("development")),
            traces_sample_rate: std::env::var("RUNESH_SAMPLE_RATE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            debug: matches!(
                std::env::var("RUNESH_TELEMETRY_DEBUG").as_deref(),
                Ok("1") | Ok("true")
            ),
        }
    }
}

/// Initialize telemetry. The returned guard must be kept alive for the lifetime
/// of the program (typically bound in `main`).
///
/// If `config.dsn` is `None`, this returns a no-op guard and does nothing —
/// safe to call unconditionally in every binary.
pub fn init(config: Config) -> Option<TelemetryGuard> {
    let dsn = config.dsn.as_deref()?;

    let release = format!("{}@{}", config.service, config.version);

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: Some(Cow::Owned(release)),
            environment: Some(config.environment.clone()),
            traces_sample_rate: config.traces_sample_rate,
            debug: config.debug,
            send_default_pii: false,
            attach_stacktrace: true,
            ..Default::default()
        },
    ));

    tracing::info!(
        service = %config.service,
        version = %config.version,
        env = %config.environment,
        "runesh-telemetry initialized"
    );

    Some(guard)
}

/// Build a `tracing-subscriber` layer that forwards `tracing` events to Sentry.
///
/// `WARN` and `ERROR` events become breadcrumbs/events. Span tracking is
/// disabled because sentry-tracing 0.47's `HubSwitchGuard` is not safe across
/// tokio worker threads — futures get moved between workers, the enter/exit
/// counts don't match, and the layer panics on span exit. Dropping spans
/// preserves the important behaviour (errors still flow) without the foot-gun.
/// See: <https://github.com/getsentry/sentry-rust/issues/737>
#[cfg(feature = "tracing-layer")]
pub fn tracing_layer<S>() -> sentry_tracing::SentryLayer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    sentry_tracing::layer().span_filter(|_| false)
}

/// Axum / Tower middleware helpers.
#[cfg(feature = "axum")]
pub mod axum {
    /// Returns a Tower `Layer` that captures request context (URL, method,
    /// headers) on every request and attaches it to any Sentry events emitted
    /// while the request is being handled.
    pub fn layer() -> sentry_tower::SentryHttpLayer {
        sentry_tower::SentryHttpLayer::new().enable_transaction()
    }

    /// Re-export of `sentry_tower::NewSentryLayer` — install this *before*
    /// [`layer`] to scope hub state per request (recommended).
    pub use sentry_tower::NewSentryLayer;
}

/// Manually capture an error. Most code should prefer `tracing::error!` and
/// rely on the [`tracing_layer`] to forward it automatically.
pub fn capture_error<E: std::error::Error + ?Sized>(err: &E) -> sentry::types::Uuid {
    sentry::capture_error(err)
}

/// Manually capture a message.
pub fn capture_message(msg: &str, level: sentry::Level) -> sentry::types::Uuid {
    sentry::capture_message(msg, level)
}

pub use sentry::Level;
