//! Prometheus metrics support.
//!
//! Provides a `setup_metrics()` function to install a Prometheus recorder and
//! a `metrics_handler()` Axum handler to expose the `/metrics` endpoint.
//!
//! Requires the `metrics` feature flag.
//!
//! # Usage
//!
//! ```ignore
//! use runesh_core::metrics::{setup_metrics, metrics_handler};
//!
//! // During startup, install the recorder:
//! setup_metrics();
//!
//! // Mount the handler:
//! let app = Router::new()
//!     .route("/metrics", get(metrics_handler));
//! ```

use axum::http::StatusCode;
use axum::response::IntoResponse;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;

static PROM_HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

/// Install the Prometheus metrics recorder.
///
/// Call this once during application startup. Subsequent calls are no-ops.
/// After this is called, all `metrics::counter!`, `metrics::histogram!`, and
/// `metrics::gauge!` macros will record to the Prometheus exporter.
pub fn setup_metrics() {
    PROM_HANDLE.get_or_init(|| {
        let builder = PrometheusBuilder::new();
        let handle = builder
            .install_recorder()
            .expect("failed to install Prometheus recorder");
        tracing::info!("Prometheus metrics recorder installed");
        handle
    });
}

/// Axum handler that renders collected metrics in Prometheus text format.
///
/// Mount at `/metrics`:
/// ```ignore
/// Router::new().route("/metrics", get(metrics_handler))
/// ```
pub async fn metrics_handler() -> impl IntoResponse {
    match PROM_HANDLE.get() {
        Some(handle) => (StatusCode::OK, handle.render()),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "metrics recorder not initialized".to_string(),
        ),
    }
}
