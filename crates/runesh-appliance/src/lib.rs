#![deny(unsafe_code)]
//! Uniform network appliance driver trait.
//!
//! Every network device driver implements the same trait surface:
//! identity, inventory, config, firewall, health, lifecycle.

pub mod opnsense;
pub mod unifi;

pub use opnsense::OPNsenseDriver;
pub use unifi::UniFiDriver;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretBox, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Appliance connection credentials.
///
/// Secret-bearing fields are wrapped in `SecretString` / `SecretBox<Vec<u8>>`
/// so they are zeroed on drop and redacted in `Debug` output. Serde
/// round-trips still work for on-disk persistence: the fields expose their
/// values only when explicitly serialized.
pub enum Credentials {
    /// API key + secret (OPNsense).
    ApiKeySecret { key: String, secret: SecretString },
    /// Bearer token (FortiGate).
    BearerToken { token: SecretString },
    /// Username + password (MikroTik, generic).
    UsernamePassword {
        username: String,
        password: SecretString,
    },
    /// SSH key (Cisco, Juniper). The raw PEM bytes are held as a secret.
    SshKey {
        username: String,
        private_key: SecretBox<Vec<u8>>,
    },
    /// SNMP community string (v2c).
    SnmpV2c { community: SecretString },
    /// SNMP v3 credentials.
    SnmpV3 {
        username: String,
        auth_pass: SecretString,
        priv_pass: SecretString,
    },
    /// HTTP header (UniFi X-API-Key). `value` is treated as secret.
    HttpHeader { header: String, value: SecretString },
}

impl Clone for Credentials {
    fn clone(&self) -> Self {
        match self {
            Self::ApiKeySecret { key, secret } => Self::ApiKeySecret {
                key: key.clone(),
                secret: SecretString::from(secret.expose_secret().to_string()),
            },
            Self::BearerToken { token } => Self::BearerToken {
                token: SecretString::from(token.expose_secret().to_string()),
            },
            Self::UsernamePassword { username, password } => Self::UsernamePassword {
                username: username.clone(),
                password: SecretString::from(password.expose_secret().to_string()),
            },
            Self::SshKey {
                username,
                private_key,
            } => Self::SshKey {
                username: username.clone(),
                private_key: SecretBox::new(Box::new(private_key.expose_secret().clone())),
            },
            Self::SnmpV2c { community } => Self::SnmpV2c {
                community: SecretString::from(community.expose_secret().to_string()),
            },
            Self::SnmpV3 {
                username,
                auth_pass,
                priv_pass,
            } => Self::SnmpV3 {
                username: username.clone(),
                auth_pass: SecretString::from(auth_pass.expose_secret().to_string()),
                priv_pass: SecretString::from(priv_pass.expose_secret().to_string()),
            },
            Self::HttpHeader { header, value } => Self::HttpHeader {
                header: header.clone(),
                value: SecretString::from(value.expose_secret().to_string()),
            },
        }
    }
}

/// Custom Debug prints only field names; secret values are always elided.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKeySecret { key, .. } => f
                .debug_struct("Credentials::ApiKeySecret")
                .field("key", key)
                .field("secret", &"<redacted>")
                .finish(),
            Self::BearerToken { .. } => f
                .debug_struct("Credentials::BearerToken")
                .field("token", &"<redacted>")
                .finish(),
            Self::UsernamePassword { username, .. } => f
                .debug_struct("Credentials::UsernamePassword")
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            Self::SshKey { username, .. } => f
                .debug_struct("Credentials::SshKey")
                .field("username", username)
                .field("private_key", &"<redacted>")
                .finish(),
            Self::SnmpV2c { .. } => f
                .debug_struct("Credentials::SnmpV2c")
                .field("community", &"<redacted>")
                .finish(),
            Self::SnmpV3 { username, .. } => f
                .debug_struct("Credentials::SnmpV3")
                .field("username", username)
                .field("auth_pass", &"<redacted>")
                .field("priv_pass", &"<redacted>")
                .finish(),
            Self::HttpHeader { header, .. } => f
                .debug_struct("Credentials::HttpHeader")
                .field("header", header)
                .field("value", &"<redacted>")
                .finish(),
        }
    }
}

/// Plaintext serde shape used only for on-disk persistence.
/// This mirrors the old derive-based `Credentials` so existing
/// config files and API bodies keep working.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CredentialsWire {
    ApiKeySecret {
        key: String,
        secret: String,
    },
    BearerToken {
        token: String,
    },
    UsernamePassword {
        username: String,
        password: String,
    },
    SshKey {
        username: String,
        private_key: String,
    },
    SnmpV2c {
        community: String,
    },
    SnmpV3 {
        username: String,
        auth_pass: String,
        priv_pass: String,
    },
    HttpHeader {
        header: String,
        value: String,
    },
}

impl Serialize for Credentials {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::ApiKeySecret { key, secret } => CredentialsWire::ApiKeySecret {
                key: key.clone(),
                secret: secret.expose_secret().to_string(),
            },
            Self::BearerToken { token } => CredentialsWire::BearerToken {
                token: token.expose_secret().to_string(),
            },
            Self::UsernamePassword { username, password } => CredentialsWire::UsernamePassword {
                username: username.clone(),
                password: password.expose_secret().to_string(),
            },
            Self::SshKey {
                username,
                private_key,
            } => CredentialsWire::SshKey {
                username: username.clone(),
                private_key: String::from_utf8(private_key.expose_secret().clone())
                    .unwrap_or_default(),
            },
            Self::SnmpV2c { community } => CredentialsWire::SnmpV2c {
                community: community.expose_secret().to_string(),
            },
            Self::SnmpV3 {
                username,
                auth_pass,
                priv_pass,
            } => CredentialsWire::SnmpV3 {
                username: username.clone(),
                auth_pass: auth_pass.expose_secret().to_string(),
                priv_pass: priv_pass.expose_secret().to_string(),
            },
            Self::HttpHeader { header, value } => CredentialsWire::HttpHeader {
                header: header.clone(),
                value: value.expose_secret().to_string(),
            },
        };
        wire.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for Credentials {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let wire = CredentialsWire::deserialize(de)?;
        Ok(match wire {
            CredentialsWire::ApiKeySecret { key, secret } => Self::ApiKeySecret {
                key,
                secret: SecretString::from(secret),
            },
            CredentialsWire::BearerToken { token } => Self::BearerToken {
                token: SecretString::from(token),
            },
            CredentialsWire::UsernamePassword { username, password } => Self::UsernamePassword {
                username,
                password: SecretString::from(password),
            },
            CredentialsWire::SshKey {
                username,
                private_key,
            } => Self::SshKey {
                username,
                private_key: SecretBox::new(Box::new(private_key.into_bytes())),
            },
            CredentialsWire::SnmpV2c { community } => Self::SnmpV2c {
                community: SecretString::from(community),
            },
            CredentialsWire::SnmpV3 {
                username,
                auth_pass,
                priv_pass,
            } => Self::SnmpV3 {
                username,
                auth_pass: SecretString::from(auth_pass),
                priv_pass: SecretString::from(priv_pass),
            },
            CredentialsWire::HttpHeader { header, value } => Self::HttpHeader {
                header,
                value: SecretString::from(value),
            },
        })
    }
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

    /// Apply a configuration delta to the running device.
    ///
    /// Drivers that can do structured config application (OPNsense) diff
    /// against current state and POST the result. Drivers without an
    /// equivalent capability return `NotSupported`.
    async fn apply_config(&mut self, _config: serde_json::Value) -> Result<(), ApplianceError> {
        Err(ApplianceError::NotSupported("apply_config".into()))
    }

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
            secret: SecretString::from("xyz".to_string()),
        };
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("api_key_secret"));
        // Values MUST be present when explicitly serialized for persistence.
        assert!(json.contains("xyz"));

        let creds2 = Credentials::BearerToken {
            token: SecretString::from("tok".to_string()),
        };
        let json2 = serde_json::to_string(&creds2).unwrap();
        let parsed: Credentials = serde_json::from_str(&json2).unwrap();
        assert!(matches!(parsed, Credentials::BearerToken { .. }));
    }

    #[test]
    fn debug_never_leaks_secrets() {
        let creds = Credentials::ApiKeySecret {
            key: "visible-key".into(),
            secret: SecretString::from("SUPER_SECRET_DO_NOT_LOG".to_string()),
        };
        let dbg = format!("{creds:?}");
        assert!(dbg.contains("visible-key"));
        assert!(!dbg.contains("SUPER_SECRET_DO_NOT_LOG"));
        assert!(dbg.contains("<redacted>"));

        let pw = Credentials::UsernamePassword {
            username: "alice".into(),
            password: SecretString::from("hunter2".to_string()),
        };
        let dbg = format!("{pw:?}");
        assert!(dbg.contains("alice"));
        assert!(!dbg.contains("hunter2"));

        let snmp = Credentials::SnmpV3 {
            username: "u".into(),
            auth_pass: SecretString::from("AUTH_LEAK".to_string()),
            priv_pass: SecretString::from("PRIV_LEAK".to_string()),
        };
        let dbg = format!("{snmp:?}");
        assert!(!dbg.contains("AUTH_LEAK"));
        assert!(!dbg.contains("PRIV_LEAK"));
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
                secret: SecretString::from("s".to_string()),
            },
            Credentials::BearerToken {
                token: SecretString::from("t".to_string()),
            },
            Credentials::UsernamePassword {
                username: "u".into(),
                password: SecretString::from("p".to_string()),
            },
            Credentials::SshKey {
                username: "u".into(),
                private_key: SecretBox::new(Box::new(b"k".to_vec())),
            },
            Credentials::SnmpV2c {
                community: SecretString::from("public".to_string()),
            },
            Credentials::SnmpV3 {
                username: "u".into(),
                auth_pass: SecretString::from("a".to_string()),
                priv_pass: SecretString::from("p".to_string()),
            },
            Credentials::HttpHeader {
                header: "X-API-Key".into(),
                value: SecretString::from("v".to_string()),
            },
        ];
        for cred in types {
            let json = serde_json::to_string(&cred).unwrap();
            let _parsed: Credentials = serde_json::from_str(&json).unwrap();
        }
    }
}
