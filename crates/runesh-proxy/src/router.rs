//! Request routing engine.
//!
//! Resolves an incoming request to a backend target by matching
//! the hostname and applying load balancing.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ProxyError;
use crate::config::{Backend, LoadBalance, ProxyConfig};

/// A resolved route: the backend to forward to.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    /// The matched resource.
    pub resource_id: String,
    /// Tenant that owns this resource.
    pub tenant_id: String,
    /// Backend address to connect to.
    pub backend_addr: String,
    /// Backend port.
    pub backend_port: u16,
    /// Whether to use TLS to the backend.
    pub backend_tls: bool,
}

/// Routes requests to backends based on hostname matching and load balancing.
pub struct Router {
    config: ProxyConfig,
    /// Per-resource round-robin counters.
    rr_counters: std::collections::HashMap<String, AtomicUsize>,
}

impl Router {
    pub fn new(config: ProxyConfig) -> Self {
        let mut rr_counters = std::collections::HashMap::new();
        for (hostname, _) in &config.resources {
            rr_counters.insert(hostname.clone(), AtomicUsize::new(0));
        }
        Self {
            config,
            rr_counters,
        }
    }

    /// Update the configuration.
    pub fn update_config(&mut self, config: ProxyConfig) {
        // Preserve existing counters, add new ones
        for (hostname, _) in &config.resources {
            self.rr_counters
                .entry(hostname.clone())
                .or_insert_with(|| AtomicUsize::new(0));
        }
        // Remove counters for removed resources
        self.rr_counters
            .retain(|k, _| config.resources.contains_key(k));
        self.config = config;
    }

    /// Resolve a request to a backend.
    pub fn resolve(&self, hostname: &str) -> Result<ResolvedRoute, ProxyError> {
        let resource = self
            .config
            .route(hostname)
            .ok_or_else(|| ProxyError::NoRoute(hostname.to_string()))?;

        if !resource.enabled {
            return Err(ProxyError::NoRoute(format!("{hostname} (disabled)")));
        }

        let backends = resource.healthy_backends();
        if backends.is_empty() {
            return Err(ProxyError::BackendUnreachable(format!(
                "no healthy backends for {hostname}"
            )));
        }

        let backend = self.pick_backend(hostname, &backends, &resource.load_balance);

        Ok(ResolvedRoute {
            resource_id: resource.id.clone(),
            tenant_id: resource.tenant_id.clone(),
            backend_addr: backend.address.clone(),
            backend_port: backend.port,
            backend_tls: backend.tls,
        })
    }

    /// Pick a backend using the configured load balancing strategy.
    fn pick_backend<'a>(
        &self,
        hostname: &str,
        backends: &[&'a Backend],
        strategy: &LoadBalance,
    ) -> &'a Backend {
        match strategy {
            LoadBalance::RoundRobin => {
                let counter = self
                    .rr_counters
                    .get(hostname)
                    .expect("counter should exist");
                let idx = counter.fetch_add(1, Ordering::Relaxed) % backends.len();
                backends[idx]
            }
            LoadBalance::Random => {
                let idx = rand_index(backends.len());
                backends[idx]
            }
            LoadBalance::IpHash | LoadBalance::LeastConn => {
                // Fallback to round-robin for now
                let counter = self
                    .rr_counters
                    .get(hostname)
                    .expect("counter should exist");
                let idx = counter.fetch_add(1, Ordering::Relaxed) % backends.len();
                backends[idx]
            }
        }
    }

    /// Get the underlying config.
    pub fn config(&self) -> &ProxyConfig {
        &self.config
    }
}

/// Simple pseudo-random index (no external dep needed for this).
fn rand_index(max: usize) -> usize {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    nanos % max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn make_config() -> ProxyConfig {
        let mut config = ProxyConfig::default();
        config.upsert(Resource {
            id: "res-1".into(),
            tenant_id: "t1".into(),
            hostname: "app.example.com".into(),
            public_port: 443,
            protocol: Protocol::Https,
            backends: vec![
                Backend {
                    address: "100.64.0.10".into(),
                    port: 8080,
                    tls: false,
                    weight: 1,
                    healthy: true,
                },
                Backend {
                    address: "100.64.0.11".into(),
                    port: 8080,
                    tls: false,
                    weight: 1,
                    healthy: true,
                },
            ],
            load_balance: LoadBalance::RoundRobin,
            tls: TlsConfig::default(),
            access: AccessConfig::default(),
            http: None,
            enabled: true,
        });
        config
    }

    #[test]
    fn resolve_known_host() {
        let router = Router::new(make_config());
        let route = router.resolve("app.example.com").unwrap();
        assert_eq!(route.tenant_id, "t1");
        assert_eq!(route.backend_port, 8080);
    }

    #[test]
    fn resolve_unknown_host() {
        let router = Router::new(make_config());
        assert!(router.resolve("unknown.example.com").is_err());
    }

    #[test]
    fn round_robin_distributes() {
        let router = Router::new(make_config());
        let r1 = router.resolve("app.example.com").unwrap();
        let r2 = router.resolve("app.example.com").unwrap();
        // Should alternate between the two backends
        assert_ne!(r1.backend_addr, r2.backend_addr);
    }

    #[test]
    fn no_healthy_backends() {
        let mut config = ProxyConfig::default();
        config.upsert(Resource {
            id: "res-1".into(),
            tenant_id: "t1".into(),
            hostname: "down.example.com".into(),
            public_port: 443,
            protocol: Protocol::Https,
            backends: vec![Backend {
                address: "100.64.0.10".into(),
                port: 8080,
                tls: false,
                weight: 1,
                healthy: false,
            }],
            load_balance: LoadBalance::default(),
            tls: TlsConfig::default(),
            access: AccessConfig::default(),
            http: None,
            enabled: true,
        });
        let router = Router::new(config);
        assert!(router.resolve("down.example.com").is_err());
    }

    #[test]
    fn disabled_resource() {
        let mut config = make_config();
        config.resources.get_mut("app.example.com").unwrap().enabled = false;
        let router = Router::new(config);
        assert!(router.resolve("app.example.com").is_err());
    }

    #[test]
    fn config_update() {
        let mut router = Router::new(make_config());
        assert!(router.resolve("app.example.com").is_ok());

        // Update with empty config
        router.update_config(ProxyConfig::default());
        assert!(router.resolve("app.example.com").is_err());
    }
}
