//! # runesh-telemetry
//!
//! Sentry / GlitchTip error reporting for RUNESH services.
//!
//! GlitchTip is wire-compatible with the Sentry SDK protocol, so the official
//! `sentry` crate works against either backend with no code changes. Point
//! `RUNESH_SENTRY_DSN` at your GlitchTip instance and the same init code works.
//!
//! ## Sensitive header redaction
//!
//! The Tower/Axum layer attaches incoming request headers to every captured
//! event. That includes `Authorization`, `Cookie`, and a handful of vendor
//! API-key headers that must never land in an error dashboard. The
//! [`SensitiveHeaderPolicy`] installed by default strips (or redacts to
//! `"[redacted]"`) a conservative list of headers from both request and
//! response contexts and from breadcrumb data on every Sentry event. Callers
//! can extend the list or replace the policy entirely via [`Config::redact_headers`].
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
//! - `RUNESH_SENTRY_DSN` - DSN string. If unset, telemetry is disabled (no-op).
//! - `RUNESH_ENV` - environment name (e.g. "production", "staging"). Default: "development".
//! - `RUNESH_SAMPLE_RATE` - performance traces sample rate, `0.0..1.0`. Default: `0.0` (errors only).
//! - `RUNESH_TELEMETRY_DEBUG` - set to `1` to log Sentry's own debug output.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

pub use sentry::ClientInitGuard as TelemetryGuard;

/// Policy describing which HTTP header names must be stripped from Sentry
/// events before they leave the process. All comparisons are case-insensitive.
#[derive(Debug, Clone)]
pub struct SensitiveHeaderPolicy {
    /// Header names to strip, stored in lowercase for O(log n) case-insensitive
    /// membership tests.
    strip: BTreeSet<String>,
}

impl SensitiveHeaderPolicy {
    /// Default strip list covering the common authorization and vendor-API
    /// headers. Extend via [`add`](Self::add) or replace via [`with_strip`](Self::with_strip).
    pub fn default_strip() -> Self {
        let mut strip = BTreeSet::new();
        for name in [
            "authorization",
            "cookie",
            "set-cookie",
            "proxy-authorization",
            "x-api-key",
            "x-auth-token",
            "x-amz-security-token",
        ] {
            strip.insert(name.to_string());
        }
        Self { strip }
    }

    /// Build a policy from an explicit list. Names are normalised to lowercase.
    pub fn with_strip<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let strip = names
            .into_iter()
            .map(|s| s.into().to_ascii_lowercase())
            .collect();
        Self { strip }
    }

    /// Add a header to the strip list.
    pub fn add(&mut self, name: impl Into<String>) {
        self.strip.insert(name.into().to_ascii_lowercase());
    }

    /// True if the given header should be redacted.
    pub fn is_sensitive(&self, name: &str) -> bool {
        self.strip.contains(&name.to_ascii_lowercase())
    }

    /// Names in the strip list (lowercase).
    pub fn sensitive_names(&self) -> impl Iterator<Item = &str> {
        self.strip.iter().map(String::as_str)
    }
}

impl Default for SensitiveHeaderPolicy {
    fn default() -> Self {
        Self::default_strip()
    }
}

/// Apply the policy to a sentry [`Event`], mutating any embedded request /
/// response headers and any breadcrumb payload whose `data` map carries
/// header-like keys. Returns the same event so it can be chained.
fn redact_event(
    policy: &SensitiveHeaderPolicy,
    mut event: sentry::protocol::Event<'static>,
) -> sentry::protocol::Event<'static> {
    if let Some(req) = event.request.as_mut() {
        redact_string_map(policy, &mut req.headers);
        // Cookie header is duplicated into a dedicated field on Request;
        // clear it unconditionally so we never ship session tokens.
        req.cookies = None;
    }
    for bc in event.breadcrumbs.iter_mut() {
        for (k, v) in bc.data.iter_mut() {
            if policy.is_sensitive(k) {
                *v = sentry::protocol::Value::String("[redacted]".to_string());
            }
            // Best-effort: nested maps that look like "headers".
            if k.eq_ignore_ascii_case("headers")
                && let sentry::protocol::Value::Object(map) = v
            {
                for (hk, hv) in map.iter_mut() {
                    if policy.is_sensitive(hk) {
                        *hv = sentry::protocol::Value::String("[redacted]".to_string());
                    }
                }
            }
        }
    }
    event
}

fn redact_string_map<M>(policy: &SensitiveHeaderPolicy, map: &mut M)
where
    M: HeaderMapLike,
{
    map.redact_with(|name| policy.is_sensitive(name));
}

trait HeaderMapLike {
    fn redact_with<F>(&mut self, is_sensitive: F)
    where
        F: Fn(&str) -> bool;
}

impl HeaderMapLike for std::collections::BTreeMap<String, String> {
    fn redact_with<F>(&mut self, is_sensitive: F)
    where
        F: Fn(&str) -> bool,
    {
        for (k, v) in self.iter_mut() {
            if is_sensitive(k) {
                *v = "[redacted]".to_string();
            }
        }
    }
}

/// Configuration for [`init`].
#[derive(Clone)]
pub struct Config {
    /// Sentry/GlitchTip DSN. If `None`, [`init`] is a no-op.
    pub dsn: Option<String>,
    /// Service name (typically `env!("CARGO_PKG_NAME")`).
    pub service: Cow<'static, str>,
    /// Service version (typically `env!("CARGO_PKG_VERSION")`).
    pub version: Cow<'static, str>,
    /// Deployment environment ("production", "staging", "development", ...).
    pub environment: Cow<'static, str>,
    /// Performance traces sample rate (`0.0..1.0`). `0.0` disables tracing entirely.
    pub traces_sample_rate: f32,
    /// Enable Sentry's internal debug logging.
    pub debug: bool,
    /// Header redaction policy. Default: [`SensitiveHeaderPolicy::default_strip`].
    pub redact_headers: SensitiveHeaderPolicy,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("dsn", &self.dsn.as_deref().map(|_| "<set>"))
            .field("service", &self.service)
            .field("version", &self.version)
            .field("environment", &self.environment)
            .field("traces_sample_rate", &self.traces_sample_rate)
            .field("debug", &self.debug)
            .field(
                "redact_headers",
                &self.redact_headers.sensitive_names().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Config {
    /// Build a config from `RUNESH_*` environment variables.
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
            redact_headers: SensitiveHeaderPolicy::default_strip(),
        }
    }
}

/// Initialize telemetry. The returned guard must be kept alive for the lifetime
/// of the program (typically bound in `main`).
///
/// If `config.dsn` is `None`, this returns a no-op guard and does nothing,
/// so it is safe to call unconditionally in every binary.
pub fn init(config: Config) -> Option<TelemetryGuard> {
    let dsn = config.dsn.as_deref()?;

    let release = format!("{}@{}", config.service, config.version);
    let policy = Arc::new(config.redact_headers.clone());
    let before_send_policy = policy.clone();

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: Some(Cow::Owned(release)),
            environment: Some(config.environment.clone()),
            traces_sample_rate: config.traces_sample_rate,
            debug: config.debug,
            send_default_pii: false,
            attach_stacktrace: true,
            before_send: Some(Arc::new(move |event| {
                Some(redact_event(&before_send_policy, event))
            })),
            ..Default::default()
        },
    ));

    tracing::info!(
        service = %config.service,
        version = %config.version,
        env = %config.environment,
        redacted_headers = ?policy.sensitive_names().collect::<Vec<_>>(),
        "runesh-telemetry initialized"
    );

    Some(guard)
}

/// Directly apply a policy to an event. Exposed for tests and for callers
/// that want to run the redaction pass manually (e.g. before forwarding an
/// event to a secondary sink).
pub fn redact_event_for_tests(
    policy: &SensitiveHeaderPolicy,
    event: sentry::protocol::Event<'static>,
) -> sentry::protocol::Event<'static> {
    redact_event(policy, event)
}

/// Build a `tracing-subscriber` layer that forwards `tracing` events to Sentry.
///
/// `WARN` and `ERROR` events become breadcrumbs/events. Span tracking is
/// disabled because sentry-tracing 0.47's `HubSwitchGuard` is not safe across
/// tokio worker threads; futures get moved between workers, the enter/exit
/// counts do not match, and the layer panics on span exit. Dropping spans
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
    /// while the request is being handled. Sensitive headers are redacted by
    /// the `before_send` hook installed by [`super::init`].
    pub fn layer() -> sentry_tower::SentryHttpLayer {
        sentry_tower::SentryHttpLayer::new().enable_transaction()
    }

    /// Re-export of `sentry_tower::NewSentryLayer`; install this *before*
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

#[cfg(test)]
mod tests {
    use super::*;
    use sentry::protocol::{Breadcrumb, Event, Request};

    #[test]
    fn default_policy_includes_common_headers() {
        let p = SensitiveHeaderPolicy::default_strip();
        for h in [
            "authorization",
            "Authorization",
            "COOKIE",
            "x-api-key",
            "X-Amz-Security-Token",
        ] {
            assert!(p.is_sensitive(h), "header {h} should be flagged sensitive");
        }
    }

    #[test]
    fn policy_case_insensitive() {
        let p = SensitiveHeaderPolicy::with_strip(["My-Token"]);
        assert!(p.is_sensitive("my-token"));
        assert!(p.is_sensitive("MY-TOKEN"));
        assert!(!p.is_sensitive("other"));
    }

    #[test]
    fn redacts_request_headers() {
        let policy = SensitiveHeaderPolicy::default_strip();
        let mut req = Request::default();
        req.headers
            .insert("authorization".into(), "Bearer secret".into());
        req.headers
            .insert("x-forwarded-for".into(), "1.2.3.4".into());
        req.cookies = Some("session=abc".into());

        let mut event = Event::new();
        event.request = Some(req);

        let out = redact_event(&policy, event);
        let req = out.request.unwrap();
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("[redacted]")
        );
        assert_eq!(
            req.headers.get("x-forwarded-for").map(String::as_str),
            Some("1.2.3.4")
        );
        assert!(req.cookies.is_none());
    }

    #[test]
    fn redacts_breadcrumb_headers() {
        let policy = SensitiveHeaderPolicy::default_strip();
        let mut bc = Breadcrumb::default();
        bc.data.insert(
            "headers".into(),
            serde_json::json!({
                "Authorization": "Bearer xyz",
                "User-Agent": "curl/8",
            }),
        );
        bc.data.insert(
            "authorization".into(),
            sentry::protocol::Value::String("top".into()),
        );

        let mut event = Event::new();
        event.breadcrumbs.values.push(bc);

        let out = redact_event(&policy, event);
        let bc = out.breadcrumbs.values.first().unwrap();
        let headers = bc.data.get("headers").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            headers.get("Authorization").and_then(|v| v.as_str()),
            Some("[redacted]")
        );
        assert_eq!(
            headers.get("User-Agent").and_then(|v| v.as_str()),
            Some("curl/8")
        );
        assert_eq!(
            bc.data.get("authorization").and_then(|v| v.as_str()),
            Some("[redacted]")
        );
    }

    #[test]
    fn custom_policy_replaces_defaults() {
        let policy = SensitiveHeaderPolicy::with_strip(["my-secret"]);
        let mut req = Request::default();
        req.headers
            .insert("authorization".into(), "Bearer keep".into());
        req.headers.insert("my-secret".into(), "hide".into());
        let mut event = Event::new();
        event.request = Some(req);
        let out = redact_event(&policy, event);
        let req = out.request.unwrap();
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer keep")
        );
        assert_eq!(
            req.headers.get("my-secret").map(String::as_str),
            Some("[redacted]")
        );
    }
}
