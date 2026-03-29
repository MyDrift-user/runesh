//! Graceful shutdown signal for containerized deployments.
//!
//! Provides a shutdown signal that waits for Ctrl+C or SIGTERM, then runs
//! registered cleanup hooks and drains in-flight requests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// A boxed async cleanup function.
type ShutdownHook = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Registry for shutdown cleanup tasks.
///
/// Consumers can register hooks that run during graceful shutdown,
/// e.g. flushing buffers, closing connections, or deregistering from service discovery.
///
/// ```ignore
/// let registry = ShutdownRegistry::new();
/// registry.register("flush metrics", || Box::pin(async {
///     // flush pending metrics
/// })).await;
///
/// // Later, pass to graceful_shutdown:
/// graceful_shutdown(registry, Duration::from_secs(15)).await;
/// ```
pub struct ShutdownRegistry {
    hooks: Arc<Mutex<Vec<(String, ShutdownHook)>>>,
}

impl ShutdownRegistry {
    /// Create an empty shutdown registry.
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register a named cleanup task to run during shutdown.
    ///
    /// Hooks run sequentially in registration order.
    pub async fn register<F, Fut>(&self, name: impl Into<String>, hook: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let name = name.into();
        tracing::debug!(hook = %name, "registered shutdown hook");
        let boxed: ShutdownHook = Box::new(move || Box::pin(hook()));
        self.hooks.lock().await.push((name, boxed));
    }

    /// Run all registered hooks sequentially.
    async fn run_hooks(self) {
        let hooks = self.hooks.lock().await.drain(..).collect::<Vec<_>>();
        let count = hooks.len();
        if count > 0 {
            tracing::info!(count, "running shutdown hooks");
        }
        for (name, hook) in hooks {
            tracing::info!(hook = %name, "running shutdown hook");
            (hook)().await;
            tracing::info!(hook = %name, "shutdown hook completed");
        }
    }
}

impl Default for ShutdownRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for a shutdown signal (Ctrl+C or SIGTERM).
///
/// Usage with Axum:
/// ```ignore
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
/// axum::serve(listener, app)
///     .with_graceful_shutdown(shutdown_signal())
///     .await?;
/// ```
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Ctrl+C received, shutting down"),
        _ = terminate => tracing::info!("SIGTERM received, shutting down"),
    }
}

/// Enhanced graceful shutdown with drain period and cleanup hooks.
///
/// 1. Waits for Ctrl+C or SIGTERM.
/// 2. Runs all registered shutdown hooks.
/// 3. Waits for `drain_period` to let in-flight requests complete.
///
/// ```ignore
/// let registry = ShutdownRegistry::new();
/// registry.register("close db", || Box::pin(async { pool.close().await })).await;
///
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
/// axum::serve(listener, app)
///     .with_graceful_shutdown(graceful_shutdown(registry, Duration::from_secs(15)))
///     .await?;
/// ```
pub async fn graceful_shutdown(registry: ShutdownRegistry, drain_period: Duration) {
    shutdown_signal().await;

    tracing::info!("starting graceful shutdown sequence");

    // Run cleanup hooks
    registry.run_hooks().await;

    // Drain period: allow in-flight requests to complete
    tracing::info!(seconds = drain_period.as_secs(), "draining in-flight requests");
    tokio::time::sleep(drain_period).await;

    tracing::info!("graceful shutdown complete");
}
