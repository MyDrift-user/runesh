//! Proxy resource and route configuration.
//!
//! A "resource" is a published service: a hostname mapped to one or more
//! backend targets, with access control policies applied in layers.
//!
//! Resources are declared per-tenant. The proxy routes incoming requests
//! by matching the TLS SNI / Host header against the resource's public
//! hostname, then forwards to the backend via the WireGuard mesh.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The full proxy configuration: all tenants' resources.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Resources indexed by public hostname.
    pub resources: HashMap<String, Resource>,
}

/// A published resource (one hostname/port combination).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Unique resource ID.
    pub id: String,

    /// Tenant that owns this resource.
    pub tenant_id: String,

    /// Public hostname (matched against SNI / Host header).
    pub hostname: String,

    /// Public-side port (typically 443 for HTTPS, or a custom port for TCP).
    #[serde(default = "default_public_port")]
    pub public_port: u16,

    /// Protocol for this resource.
    #[serde(default)]
    pub protocol: Protocol,

    /// Backend targets (mesh IPs or hostnames + ports).
    pub backends: Vec<Backend>,

    /// Load balancing strategy.
    #[serde(default)]
    pub load_balance: LoadBalance,

    /// TLS configuration.
    #[serde(default)]
    pub tls: TlsConfig,

    /// Access control layers.
    #[serde(default)]
    pub access: AccessConfig,

    /// HTTP-specific options (path rewrites, headers, etc.).
    #[serde(default)]
    pub http: Option<HttpConfig>,

    /// Whether this resource is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Protocol type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Https,
    Http,
    Tcp,
    Udp,
    Grpc,
}

/// A backend target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    /// Backend address (mesh IP or hostname).
    pub address: String,

    /// Backend port.
    pub port: u16,

    /// Whether to use TLS to the backend.
    #[serde(default)]
    pub tls: bool,

    /// Weight for load balancing (higher = more traffic).
    #[serde(default = "default_weight")]
    pub weight: u32,

    /// Whether this backend is healthy.
    #[serde(default = "default_true")]
    pub healthy: bool,
}

/// Load balancing strategy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalance {
    #[default]
    RoundRobin,
    LeastConn,
    Random,
    IpHash,
}

/// TLS configuration for a resource.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Whether ACME auto-provisioning is enabled.
    #[serde(default = "default_true")]
    pub acme: bool,

    /// ACME challenge type.
    #[serde(default)]
    pub acme_challenge: AcmeChallenge,

    /// Manual certificate (PEM, base64-encoded).
    #[serde(default)]
    pub cert_pem: Option<String>,

    /// Manual private key (PEM, base64-encoded).
    #[serde(default)]
    pub key_pem: Option<String>,
}

/// ACME challenge type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AcmeChallenge {
    /// DNS-01 via Cloudflare API (recommended).
    #[default]
    Dns01,
    /// TLS-ALPN-01 (requires port 443 on this node).
    TlsAlpn01,
}

/// Access control configuration (4 layers from the Helvetia feature catalog).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessConfig {
    /// Layer 1: Network filter (geo, ASN, IP allow/deny, rate limit).
    #[serde(default)]
    pub network: NetworkFilter,

    /// Layer 2: Time and context (business hours, maintenance mode).
    #[serde(default)]
    pub time: Option<TimeFilter>,

    /// Layer 3: Identity gate (auth mode).
    #[serde(default)]
    pub identity: IdentityGate,

    /// Layer 4: Authorization (group, path, method, device posture).
    #[serde(default)]
    pub authz: Option<AuthzFilter>,
}

/// Layer 1: Network filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkFilter {
    /// Allowed source IPs/CIDRs (empty = allow all).
    #[serde(default)]
    pub allow_ips: Vec<String>,

    /// Denied source IPs/CIDRs.
    #[serde(default)]
    pub deny_ips: Vec<String>,

    /// Allowed countries (ISO 3166-1 alpha-2).
    #[serde(default)]
    pub allow_geo: Vec<String>,

    /// Denied countries.
    #[serde(default)]
    pub deny_geo: Vec<String>,

    /// Rate limit (requests per second per source IP, 0 = unlimited).
    #[serde(default)]
    pub rate_limit_rps: u32,
}

/// Layer 2: Time filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeFilter {
    /// Allowed time windows (cron-style, e.g., "Mon-Fri 08:00-18:00").
    #[serde(default)]
    pub windows: Vec<String>,

    /// Whether the resource is in maintenance mode.
    #[serde(default)]
    pub maintenance: bool,
}

/// Layer 3: Identity gate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdentityGate {
    /// Authentication mode.
    #[serde(default)]
    pub mode: AuthMode,
}

/// Authentication mode for a resource.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// No authentication required.
    #[default]
    Public,
    /// SSO via tenant's identity provider.
    Sso,
    /// SSO restricted to specific groups.
    Group,
    /// Mutual TLS (client certificate).
    Mtls,
    /// Pre-shared key.
    Psk,
    /// One-time link.
    OneTime,
    /// Mesh-only (only reachable from the WireGuard mesh).
    MeshOnly,
}

/// Layer 4: Authorization filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzFilter {
    /// Required groups.
    #[serde(default)]
    pub groups: Vec<String>,

    /// Allowed HTTP methods (empty = all).
    #[serde(default)]
    pub methods: Vec<String>,

    /// Allowed path prefixes (empty = all).
    #[serde(default)]
    pub paths: Vec<String>,

    /// Require step-up MFA.
    #[serde(default)]
    pub require_mfa: bool,

    /// Require specific device posture.
    #[serde(default)]
    pub posture: Vec<String>,
}

/// HTTP-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Path rewrite rules.
    #[serde(default)]
    pub rewrites: Vec<PathRewrite>,

    /// Headers to inject into upstream requests.
    #[serde(default)]
    pub request_headers: HashMap<String, String>,

    /// Headers to inject into downstream responses.
    #[serde(default)]
    pub response_headers: HashMap<String, String>,

    /// Force HSTS header.
    #[serde(default)]
    pub hsts: bool,

    /// Redirect HTTP to HTTPS.
    #[serde(default = "default_true")]
    pub force_https: bool,
}

/// Path rewrite rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRewrite {
    /// Path prefix to match.
    pub match_prefix: String,
    /// Replacement prefix.
    pub replace_prefix: String,
}

impl ProxyConfig {
    /// Look up a resource by hostname.
    pub fn route(&self, hostname: &str) -> Option<&Resource> {
        self.resources.get(hostname)
    }

    /// Add or update a resource.
    pub fn upsert(&mut self, resource: Resource) {
        self.resources.insert(resource.hostname.clone(), resource);
    }

    /// Remove a resource by hostname.
    pub fn remove(&mut self, hostname: &str) -> Option<Resource> {
        self.resources.remove(hostname)
    }

    /// Get all resources for a tenant.
    pub fn tenant_resources(&self, tenant_id: &str) -> Vec<&Resource> {
        self.resources
            .values()
            .filter(|r| r.tenant_id == tenant_id)
            .collect()
    }

    /// Get all enabled hostnames.
    pub fn hostnames(&self) -> Vec<&str> {
        self.resources
            .values()
            .filter(|r| r.enabled)
            .map(|r| r.hostname.as_str())
            .collect()
    }
}

impl Resource {
    /// Get the next healthy backend (round-robin is handled by caller).
    pub fn healthy_backends(&self) -> Vec<&Backend> {
        self.backends.iter().filter(|b| b.healthy).collect()
    }

    /// Whether this resource requires authentication.
    pub fn requires_auth(&self) -> bool {
        self.access.identity.mode != AuthMode::Public
    }

    /// Whether access is restricted to mesh-only.
    pub fn is_mesh_only(&self) -> bool {
        self.access.identity.mode == AuthMode::MeshOnly
    }

    /// Check if a source IP is allowed by the network filter.
    pub fn check_ip(&self, ip: &str) -> bool {
        let nf = &self.access.network;

        // Check deny list first
        if !nf.deny_ips.is_empty() && nf.deny_ips.iter().any(|d| d == ip) {
            return false;
        }

        // If allow list is set, IP must be in it
        if !nf.allow_ips.is_empty() {
            return nf.allow_ips.iter().any(|a| a == ip);
        }

        true
    }
}

fn default_public_port() -> u16 {
    443
}
fn default_weight() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_resource() -> Resource {
        Resource {
            id: "res-1".into(),
            tenant_id: "tenant-1".into(),
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
                    healthy: false,
                },
            ],
            load_balance: LoadBalance::RoundRobin,
            tls: TlsConfig::default(),
            access: AccessConfig {
                identity: IdentityGate {
                    mode: AuthMode::Sso,
                },
                ..Default::default()
            },
            http: Some(HttpConfig {
                hsts: true,
                force_https: true,
                ..Default::default()
            }),
            enabled: true,
        }
    }

    #[test]
    fn route_by_hostname() {
        let mut config = ProxyConfig::default();
        config.upsert(sample_resource());

        assert!(config.route("app.example.com").is_some());
        assert!(config.route("unknown.example.com").is_none());
    }

    #[test]
    fn healthy_backends_filter() {
        let res = sample_resource();
        let healthy = res.healthy_backends();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].address, "100.64.0.10");
    }

    #[test]
    fn auth_detection() {
        let res = sample_resource();
        assert!(res.requires_auth());
        assert!(!res.is_mesh_only());

        let mut public = sample_resource();
        public.access.identity.mode = AuthMode::Public;
        assert!(!public.requires_auth());
    }

    #[test]
    fn ip_filtering() {
        let mut res = sample_resource();
        res.access.network.deny_ips = vec!["10.0.0.1".into()];
        assert!(!res.check_ip("10.0.0.1"));
        assert!(res.check_ip("10.0.0.2"));

        res.access.network.deny_ips.clear();
        res.access.network.allow_ips = vec!["10.0.0.5".into()];
        assert!(res.check_ip("10.0.0.5"));
        assert!(!res.check_ip("10.0.0.6"));
    }

    #[test]
    fn tenant_resources() {
        let mut config = ProxyConfig::default();
        let mut r1 = sample_resource();
        r1.tenant_id = "t1".into();
        r1.hostname = "a.example.com".into();
        config.upsert(r1);

        let mut r2 = sample_resource();
        r2.tenant_id = "t2".into();
        r2.hostname = "b.example.com".into();
        config.upsert(r2);

        assert_eq!(config.tenant_resources("t1").len(), 1);
        assert_eq!(config.tenant_resources("t2").len(), 1);
        assert_eq!(config.tenant_resources("t3").len(), 0);
    }

    #[test]
    fn json_roundtrip() {
        let res = sample_resource();
        let json = serde_json::to_string_pretty(&res).unwrap();
        let parsed: Resource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hostname, "app.example.com");
        assert_eq!(parsed.backends.len(), 2);
        assert_eq!(parsed.access.identity.mode, AuthMode::Sso);
    }

    #[test]
    fn all_protocols() {
        for proto in [
            Protocol::Https,
            Protocol::Http,
            Protocol::Tcp,
            Protocol::Udp,
            Protocol::Grpc,
        ] {
            let json = serde_json::to_string(&proto).unwrap();
            let parsed: Protocol = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, proto);
        }
    }

    #[test]
    fn all_auth_modes() {
        for mode in [
            AuthMode::Public,
            AuthMode::Sso,
            AuthMode::Group,
            AuthMode::Mtls,
            AuthMode::Psk,
            AuthMode::OneTime,
            AuthMode::MeshOnly,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let parsed: AuthMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn hostnames_excludes_disabled() {
        let mut config = ProxyConfig::default();
        let mut r1 = sample_resource();
        r1.hostname = "enabled.example.com".into();
        r1.enabled = true;
        config.upsert(r1);

        let mut r2 = sample_resource();
        r2.hostname = "disabled.example.com".into();
        r2.enabled = false;
        config.upsert(r2);

        let hosts = config.hostnames();
        assert_eq!(hosts.len(), 1);
        assert!(hosts.contains(&"enabled.example.com"));
    }
}
