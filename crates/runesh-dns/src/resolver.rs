//! Async DNS resolver using hickory-resolver.

use crate::DnsError;
use hickory_resolver::TokioResolver;
use std::net::IpAddr;

/// Async DNS resolver.
pub struct DnsResolver {
    resolver: TokioResolver,
}

impl DnsResolver {
    /// Create a resolver using system DNS configuration.
    pub fn system() -> Result<Self, DnsError> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|e| DnsError::RecordNotFound(format!("resolver build: {e}")))?
            .build();
        Ok(Self { resolver })
    }

    /// Resolve any IP (A or AAAA) for a domain.
    pub async fn lookup_ip(&self, domain: &str) -> Result<Vec<IpAddr>, DnsError> {
        let response = self
            .resolver
            .lookup_ip(domain)
            .await
            .map_err(|e| DnsError::RecordNotFound(format!("{domain}: {e}")))?;
        Ok(response.iter().collect())
    }

    /// Resolve TXT records.
    pub async fn lookup_txt(&self, domain: &str) -> Result<Vec<String>, DnsError> {
        let response = self
            .resolver
            .txt_lookup(domain)
            .await
            .map_err(|e| DnsError::RecordNotFound(format!("{domain}: {e}")))?;
        Ok(response
            .iter()
            .map(|r| {
                r.iter()
                    .map(|d| String::from_utf8_lossy(d).to_string())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect())
    }

    /// Reverse DNS lookup.
    pub async fn reverse_lookup(&self, ip: IpAddr) -> Result<Vec<String>, DnsError> {
        let response = self
            .resolver
            .reverse_lookup(ip)
            .await
            .map_err(|e| DnsError::RecordNotFound(format!("{ip}: {e}")))?;
        Ok(response.iter().map(|n| n.to_string()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // requires network
    async fn resolve_google() {
        let resolver = DnsResolver::system().unwrap();
        let ips = resolver.lookup_ip("google.com.").await.unwrap();
        assert!(!ips.is_empty());
    }
}
