#![deny(unsafe_code)]
//! Uniform network appliance driver trait.
//!
//! Every network device driver implements the same trait surface:
//! identity, inventory, config, firewall, health, lifecycle.

pub mod opnsense;

pub use opnsense::OPNsenseDriver;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Appliance connection credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credentials {
    /// API key + secret (OPNsense).
    ApiKeySecret { key: String, secret: String },
    /// Bearer token (FortiGate).
    BearerToken { token: String },
    /// Username + password (MikroTik, generic).
    UsernamePassword { username: String, password: String },
    /// SSH key (Cisco, Juniper).
    SshKey {
        username: String,
        private_key: String,
    },
    /// SNMP community string (v2c).
    SnmpV2c { community: String },
    /// SNMP v3 credentials.
    SnmpV3 {
        username: String,
        auth_pass: String,
        priv_pass: String,
    },
    /// HTTP header (UniFi X-API-Key).
    HttpHeader { header: String, value: String },
}

/// Basic device identity information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub hostname: String,
    pub model: String,
    pub serial: String,
    pub firmware_version: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub uptime_secs: Option<u64>,
}

/// A network interface on the appliance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetInterface {
    pub name: String,
    pub mac: String,
    #[serde(default)]
    pub ipv4: Vec<String>,
    #[serde(default)]
    pub ipv6: Vec<String>,
    pub speed_mbps: Option<u32>,
    pub status: InterfaceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceStatus {
    Up,
    Down,
    Unknown,
}

/// A firewall rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallRule {
    pub id: String,
    pub action: FirewallAction,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub dst: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub protocol: String,
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirewallAction {
    Allow,
    Deny,
    Reject,
}

/// Device health metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthStatus {
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub disk_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    #[serde(default)]
    pub alerts: Vec<String>,
}

/// Configuration savepoint for rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSavepoint {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    /// Opaque config data (vendor-specific format).
    pub data: serde_json::Value,
}

/// Uniform driver trait for all network appliances.
#[async_trait]
pub trait ApplianceDriver: Send + Sync {
    /// Vendor/type name.
    fn driver_name(&self) -> &str;

    // -- Identity --

    /// Get device identity (hostname, model, serial, firmware).
    async fn get_identity(&self) -> Result<DeviceIdentity, ApplianceError>;

    // -- Inventory --

    /// List network interfaces.
    async fn list_interfaces(&self) -> Result<Vec<NetInterface>, ApplianceError>;

    // -- Config --

    /// Get the running configuration as JSON.
    async fn get_config(&self) -> Result<serde_json::Value, ApplianceError>;

    /// Create a savepoint before applying changes.
    async fn create_savepoint(&self, description: &str) -> Result<ConfigSavepoint, ApplianceError>;

    /// Rollback to a savepoint.
    async fn rollback(&self, savepoint_id: &str) -> Result<(), ApplianceError>;

    // -- Firewall --

    /// List firewall rules.
    async fn list_firewall_rules(&self) -> Result<Vec<FirewallRule>, ApplianceError>;

    /// Add a firewall rule.
    async fn add_firewall_rule(&self, rule: &FirewallRule) -> Result<String, ApplianceError>;

    /// Delete a firewall rule.
    async fn delete_firewall_rule(&self, rule_id: &str) -> Result<(), ApplianceError>;

    // -- Health --

    /// Get device health metrics.
    async fn get_health(&self) -> Result<HealthStatus, ApplianceError>;

    // -- Lifecycle --

    /// Reboot the device.
    async fn reboot(&self) -> Result<(), ApplianceError>;

    /// Check if a firmware update is available.
    async fn check_firmware_update(&self) -> Result<Option<String>, ApplianceError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ApplianceError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("API error: {0}")]
    ApiError(String),
    #[error("not supported: {0}")]
    NotSupported(String),
    #[error("timeout")]
    Timeout,
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_serialize() {
        let creds = Credentials::ApiKeySecret {
            key: "abc".into(),
            secret: "xyz".into(),
        };
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("api_key_secret"));

        let creds2 = Credentials::BearerToken {
            token: "tok".into(),
        };
        let json2 = serde_json::to_string(&creds2).unwrap();
        let parsed: Credentials = serde_json::from_str(&json2).unwrap();
        assert!(matches!(parsed, Credentials::BearerToken { .. }));
    }

    #[test]
    fn device_identity_default() {
        let id = DeviceIdentity::default();
        assert!(id.hostname.is_empty());
    }

    #[test]
    fn firewall_rule_serialize() {
        let rule = FirewallRule {
            id: "1".into(),
            action: FirewallAction::Allow,
            src: "any".into(),
            dst: "10.0.0.0/8".into(),
            port: "443".into(),
            protocol: "tcp".into(),
            enabled: true,
            description: "HTTPS".into(),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: FirewallRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.action, FirewallAction::Allow);
    }

    #[test]
    fn all_credential_types() {
        let types = vec![
            Credentials::ApiKeySecret {
                key: "k".into(),
                secret: "s".into(),
            },
            Credentials::BearerToken { token: "t".into() },
            Credentials::UsernamePassword {
                username: "u".into(),
                password: "p".into(),
            },
            Credentials::SshKey {
                username: "u".into(),
                private_key: "k".into(),
            },
            Credentials::SnmpV2c {
                community: "public".into(),
            },
            Credentials::SnmpV3 {
                username: "u".into(),
                auth_pass: "a".into(),
                priv_pass: "p".into(),
            },
            Credentials::HttpHeader {
                header: "X-API-Key".into(),
                value: "v".into(),
            },
        ];
        for cred in types {
            let json = serde_json::to_string(&cred).unwrap();
            let _parsed: Credentials = serde_json::from_str(&json).unwrap();
        }
    }
}
