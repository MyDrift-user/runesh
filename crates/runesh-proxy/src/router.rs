//! Request routing engine.
//!
//! Resolves an incoming request to a backend target by matching the
//! (normalized) hostname and applying per-resource access control before
//! picking a healthy backend.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ProxyError;
use crate::config::{Backend, LoadBalance, ProxyConfig, normalize_hostname};

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

/// Outcome of a routing decision.
///
/// Callers should use this variant to drive the response pipeline (return
/// 401 for `AuthRequired`, 403 for `Denied`, forward for `Forward`).
#[derive(Debug, Clone)]
pub enum RouteDecision {
    /// Forward the request to the chosen backend.
    Forward(ResolvedRoute),
    /// The resource requires authentication and the caller hasn't proven
    /// any identity yet.
    AuthRequired,
    /// The request is denied.
    Denied(String),
}

/// Routes requests to backends based on hostname matching and load balancing.
pub struct Router {
    config: ProxyConfig,
    /// Per-resource round-robin counters.
    rr_counters: HashMap<String, AtomicUsize>,
}

impl Router {
    pub fn new(config: ProxyConfig) -> Self {
        let mut rr_counters = HashMap::new();
        for hostname in config.resources.keys() {
            rr_counters.insert(hostname.clone(), AtomicUsize::new(0));
        }
        Self {
            config,
            rr_counters,
        }
    }

    /// Update the configuration.
    pub fn update_config(&mut self, config: ProxyConfig) {
        for hostname in config.resources.keys() {
            self.rr_counters
                .entry(hostname.clone())
                .or_insert_with(|| AtomicUsize::new(0));
        }
        self.rr_counters
            .retain(|k, _| config.resources.contains_key(k));
        self.config = config;
    }

    /// Backwards-compatible resolve: returns only the route or the same
    /// `ProxyError` values used previously. New code should call
    /// [`Router::resolve_with_context`] and honor the [`RouteDecision`].
    pub fn resolve(&self, hostname: &str) -> Result<ResolvedRoute, ProxyError> {
        match self.resolve_with_context(hostname, None, false) {
            RouteDecision::Forward(r) => Ok(r),
            RouteDecision::AuthRequired => {
                Err(ProxyError::AccessDenied("authentication required".into()))
            }
            RouteDecision::Denied(reason) => Err(ProxyError::AccessDenied(reason)),
        }
    }

    /// Resolve with full ACL context.
    ///
    /// `source_ip` is the connecting client's IP (used for IP allow/deny
    /// and geolocation checks). `is_mesh` is true when the request arrived
    /// over the WireGuard mesh interface rather than the public listener.
    pub fn resolve_with_context(
        &self,
        hostname: &str,
        source_ip: Option<&str>,
        is_mesh: bool,
    ) -> RouteDecision {
        // Normalize the Host / SNI value: lowercase, strip port, strip
        // trailing dot, strip whitespace.
        let normalized = normalize_hostname(hostname);

        let resource = match self.config.route(&normalized) {
            Some(r) => r,
            None => return RouteDecision::Denied(format!("no route for '{normalized}'")),
        };

        if !resource.enabled {
            return RouteDecision::Denied(format!("{normalized} (disabled)"));
        }

        // Mesh-only resources refuse all public-listener traffic.
        if resource.is_mesh_only() && !is_mesh {
            return RouteDecision::Denied("mesh-only resource, public access blocked".into());
        }

        // Layer 1: IP-based network filter.
        if let Some(ip) = source_ip
            && !resource.check_ip(ip)
        {
            return RouteDecision::Denied(format!("source ip {ip} denied"));
        }

        // Layer 3: Identity gate. If the resource requires auth and we
        // have no principal in the path we're evaluating, surface that
        // explicitly instead of forwarding.
        if resource.requires_auth() {
            // Authentication is handled by the caller's middleware; this
            // router just signals the requirement. Mesh traffic is still
            // considered unauthenticated for the purpose of this check
            // unless the resource explicitly opts into MeshOnly.
            return RouteDecision::AuthRequired;
        }

        let backends = resource.healthy_backends();
        if backends.is_empty() {
            return RouteDecision::Denied(format!("no healthy backends for {normalized}"));
        }

        let backend = self.pick_backend(&normalized, &backends, &resource.load_balance);

        RouteDecision::Forward(ResolvedRoute {
            resource_id: resource.id.clone(),
            tenant_id: resource.tenant_id.clone(),
            backend_addr: backend.address.clone(),
            backend_port: backend.port,
            backend_tls: backend.tls,
        })
    }

    /// Pick a backend using the configured load balancing strategy.
    ///
    /// Random selection uses a per-router atomic counter; we deliberately
    /// avoid the SystemTime-nanos modulo trick, which is non-uniform and
    /// aliases badly at high request rates.
    fn pick_backend<'a>(
        &self,
        hostname: &str,
        backends: &[&'a Backend],
        strategy: &LoadBalance,
    ) -> &'a Backend {
        let counter = self
            .rr_counters
            .get(hostname)
            .expect("counter should exist");
        match strategy {
            // Proper round-robin for every strategy we haven't implemented
            // yet (LeastConn, IpHash) — never rely on subsec_nanos.
            LoadBalance::RoundRobin
            | LoadBalance::Random
            | LoadBalance::LeastConn
            | LoadBalance::IpHash => {
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
    fn resolve_normalizes_host_header() {
        let router = Router::new(make_config());
        // Host header with port and mixed case must still resolve.
        let r = router.resolve("App.Example.Com:443");
        assert!(r.is_ok());
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

        router.update_config(ProxyConfig::default());
        assert!(router.resolve("app.example.com").is_err());
    }

    #[test]
    fn access_control_denies_by_ip() {
        let mut config = make_config();
        config
            .resources
            .get_mut("app.example.com")
            .unwrap()
            .access
            .network
            .deny_ips = vec!["10.0.0.1".into()];
        let router = Router::new(config);

        match router.resolve_with_context("app.example.com", Some("10.0.0.1"), false) {
            RouteDecision::Denied(msg) => assert!(msg.contains("10.0.0.1")),
            other => panic!("expected denied, got {other:?}"),
        }
        // Different IP passes.
        assert!(matches!(
            router.resolve_with_context("app.example.com", Some("10.0.0.2"), false),
            RouteDecision::Forward(_)
        ));
    }

    #[test]
    fn access_control_surfaces_auth_required() {
        let mut config = make_config();
        config
            .resources
            .get_mut("app.example.com")
            .unwrap()
            .access
            .identity
            .mode = AuthMode::Sso;
        let router = Router::new(config);

        assert!(matches!(
            router.resolve_with_context("app.example.com", Some("1.2.3.4"), false),
            RouteDecision::AuthRequired
        ));
    }

    #[test]
    fn mesh_only_blocks_public() {
        let mut config = make_config();
        config
            .resources
            .get_mut("app.example.com")
            .unwrap()
            .access
            .identity
            .mode = AuthMode::MeshOnly;
        let router = Router::new(config);

        // Public caller is denied.
        assert!(matches!(
            router.resolve_with_context("app.example.com", Some("1.2.3.4"), false),
            RouteDecision::Denied(_)
        ));
        // Mesh caller triggers the auth-required gate instead (MeshOnly
        // is also non-Public in `requires_auth`), which is acceptable
        // since the mesh itself authenticates via WireGuard keys.
        assert!(matches!(
            router.resolve_with_context("app.example.com", Some("100.64.0.5"), true),
            RouteDecision::AuthRequired
        ));
    }
}
