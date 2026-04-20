#![deny(unsafe_code)]
//! DNS management: MagicDNS, split DNS, zone management, service discovery,
//! and async DNS resolution via hickory-resolver.

pub mod resolver;

pub use resolver::DnsResolver;

use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

/// A DNS zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub name: String,
    pub zone_type: ZoneType,
    #[serde(default)]
    pub records: Vec<DnsRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneType {
    Internal,
    External,
    Split,
}

/// A DNS record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    pub name: String,
    pub record_type: RecordType,
    pub value: String,
    #[serde(default = "default_ttl")]
    pub ttl: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordType {
    A,
    AAAA,
    CNAME,
    MX,
    TXT,
    SRV,
    PTR,
    NS,
}

/// MagicDNS: auto-generates A records for mesh devices.
#[derive(Debug, Default)]
pub struct MagicDns {
    /// Domain suffix (e.g., "mesh.local").
    pub domain: String,
    /// hostname -> mesh IP.
    entries: HashMap<String, Ipv4Addr>,
}

impl MagicDns {
    pub fn new(domain: &str) -> Self {
        Self {
            domain: domain.to_string(),
            entries: HashMap::new(),
        }
    }

    /// Register a device. Returns the FQDN.
    pub fn register(&mut self, hostname: &str, ip: Ipv4Addr) -> String {
        let fqdn = format!("{}.{}", hostname, self.domain);
        self.entries.insert(hostname.to_string(), ip);
        fqdn
    }

    /// Remove a device.
    pub fn unregister(&mut self, hostname: &str) {
        self.entries.remove(hostname);
    }

    /// Resolve a short name or FQDN to an IP.
    pub fn resolve(&self, query: &str) -> Option<Ipv4Addr> {
        // Try exact match
        if let Some(ip) = self.entries.get(query) {
            return Some(*ip);
        }
        // Try stripping domain suffix
        if let Some(short) = query.strip_suffix(&format!(".{}", self.domain)) {
            return self.entries.get(short).copied();
        }
        None
    }

    /// Get all entries as DNS records.
    pub fn to_records(&self) -> Vec<DnsRecord> {
        self.entries
            .iter()
            .map(|(name, ip)| DnsRecord {
                name: format!("{name}.{}", self.domain),
                record_type: RecordType::A,
                value: ip.to_string(),
                ttl: 60,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Split DNS resolver: routes queries to different upstreams by domain.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SplitDnsConfig {
    /// Default upstream nameservers.
    pub default_servers: Vec<String>,
    /// Per-domain overrides.
    pub routes: HashMap<String, Vec<String>>,
}

impl SplitDnsConfig {
    /// Get the nameservers for a query domain.
    pub fn servers_for(&self, domain: &str) -> &[String] {
        // Check routes from most specific to least
        let mut candidate = domain;
        loop {
            if let Some(servers) = self.routes.get(candidate) {
                return servers;
            }
            // Strip one label
            match candidate.find('.') {
                Some(pos) => candidate = &candidate[pos + 1..],
                None => return &self.default_servers,
            }
        }
    }
}

fn default_ttl() -> u32 {
    3600
}

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("zone not found: {0}")]
    ZoneNotFound(String),
    #[error("record not found: {0}")]
    RecordNotFound(String),
    #[error("duplicate record: {0}")]
    DuplicateRecord(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_dns_register_resolve() {
        let mut dns = MagicDns::new("mesh.local");
        let fqdn = dns.register("laptop-alice", Ipv4Addr::new(100, 64, 0, 1));
        assert_eq!(fqdn, "laptop-alice.mesh.local");
        assert_eq!(
            dns.resolve("laptop-alice"),
            Some(Ipv4Addr::new(100, 64, 0, 1))
        );
        assert_eq!(
            dns.resolve("laptop-alice.mesh.local"),
            Some(Ipv4Addr::new(100, 64, 0, 1))
        );
        assert_eq!(dns.resolve("unknown"), None);
    }

    #[test]
    fn magic_dns_unregister() {
        let mut dns = MagicDns::new("mesh.local");
        dns.register("tmp", Ipv4Addr::new(100, 64, 0, 5));
        dns.unregister("tmp");
        assert_eq!(dns.resolve("tmp"), None);
    }

    #[test]
    fn magic_dns_to_records() {
        let mut dns = MagicDns::new("mesh.local");
        dns.register("a", Ipv4Addr::new(100, 64, 0, 1));
        dns.register("b", Ipv4Addr::new(100, 64, 0, 2));
        let records = dns.to_records();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.record_type == RecordType::A));
    }

    #[test]
    fn split_dns_routing() {
        let config = SplitDnsConfig {
            default_servers: vec!["1.1.1.1".into()],
            routes: HashMap::from([
                ("corp.local".into(), vec!["10.0.0.1".into()]),
                ("dev.corp.local".into(), vec!["10.0.1.1".into()]),
            ]),
        };
        assert_eq!(config.servers_for("app.corp.local"), &["10.0.0.1"]);
        assert_eq!(config.servers_for("app.dev.corp.local"), &["10.0.1.1"]);
        assert_eq!(config.servers_for("google.com"), &["1.1.1.1"]);
    }

    #[test]
    fn zone_serialization() {
        let zone = Zone {
            name: "example.com".into(),
            zone_type: ZoneType::External,
            records: vec![
                DnsRecord {
                    name: "www".into(),
                    record_type: RecordType::A,
                    value: "1.2.3.4".into(),
                    ttl: 300,
                },
                DnsRecord {
                    name: "mail".into(),
                    record_type: RecordType::MX,
                    value: "10 mail.example.com".into(),
                    ttl: 3600,
                },
            ],
        };
        let json = serde_json::to_string(&zone).unwrap();
        let parsed: Zone = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.records.len(), 2);
    }
}
