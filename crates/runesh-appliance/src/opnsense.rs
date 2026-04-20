//! OPNsense appliance driver via REST API.
//!
//! Authenticates with API key + secret via HTTP Basic auth.
//! Base URL: https://<host>/api/

use async_trait::async_trait;
use reqwest::Client;

use crate::{
    ApplianceDriver, ApplianceError, ConfigSavepoint, Credentials, DeviceIdentity, FirewallRule,
    HealthStatus, NetInterface,
};

pub struct OPNsenseDriver {
    client: Client,
    base_url: String,
    key: String,
    secret: String,
}

impl OPNsenseDriver {
    pub fn new(host: &str, credentials: &Credentials) -> Result<Self, ApplianceError> {
        let (key, secret) = match credentials {
            Credentials::ApiKeySecret { key, secret } => (key.clone(), secret.clone()),
            _ => {
                return Err(ApplianceError::AuthFailed(
                    "OPNsense requires ApiKeySecret credentials".into(),
                ));
            }
        };

        let client = Client::builder()
            .danger_accept_invalid_certs(true) // self-signed certs common on firewalls
            .build()
            .map_err(|e| ApplianceError::ConnectionFailed(e.to_string()))?;

        Ok(Self {
            client,
            base_url: format!("https://{host}/api"),
            key,
            secret,
        })
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value, ApplianceError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.key, Some(&self.secret))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(ApplianceError::ApiError(format!(
                "{} returned {}",
                path,
                resp.status()
            )));
        }

        resp.json()
            .await
            .map_err(|e| ApplianceError::ParseError(e.to_string()))
    }

    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ApplianceError> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.key, Some(&self.secret))
            .json(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(ApplianceError::ApiError(format!(
                "POST {} returned {}",
                path,
                resp.status()
            )));
        }

        resp.json()
            .await
            .map_err(|e| ApplianceError::ParseError(e.to_string()))
    }
}

#[async_trait]
impl ApplianceDriver for OPNsenseDriver {
    fn driver_name(&self) -> &str {
        "opnsense"
    }

    async fn get_identity(&self) -> Result<DeviceIdentity, ApplianceError> {
        let data = self.get("core/firmware/status").await?;
        Ok(DeviceIdentity {
            hostname: data["product_name"]
                .as_str()
                .unwrap_or("OPNsense")
                .to_string(),
            model: data["product_id"].as_str().unwrap_or("unknown").to_string(),
            serial: String::new(),
            firmware_version: data["product_version"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            vendor: "Deciso".to_string(),
            uptime_secs: None,
        })
    }

    async fn list_interfaces(&self) -> Result<Vec<NetInterface>, ApplianceError> {
        let data = self
            .get("diagnostics/interface/getInterfaceStatistics")
            .await?;
        let mut interfaces = Vec::new();

        if let Some(stats) = data["statistics"].as_object() {
            for (name, info) in stats {
                interfaces.push(NetInterface {
                    name: name.clone(),
                    mac: info["macaddr"].as_str().unwrap_or("").to_string(),
                    ipv4: info["ipv4"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v["ipaddr"].as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    ipv6: vec![],
                    speed_mbps: info["media_raw"].as_str().and_then(|s| {
                        s.split_whitespace()
                            .find_map(|w| w.strip_suffix("Mbit").and_then(|n| n.parse().ok()))
                    }),
                    status: if info["status"].as_str() == Some("up") {
                        crate::InterfaceStatus::Up
                    } else {
                        crate::InterfaceStatus::Down
                    },
                });
            }
        }

        Ok(interfaces)
    }

    async fn get_config(&self) -> Result<serde_json::Value, ApplianceError> {
        self.get("core/system/status").await
    }

    async fn create_savepoint(&self, description: &str) -> Result<ConfigSavepoint, ApplianceError> {
        let result = self
            .post("core/firmware/savepoint", &serde_json::json!({}))
            .await?;
        Ok(ConfigSavepoint {
            id: result["revision"].as_str().unwrap_or("unknown").to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            description: description.to_string(),
            data: result,
        })
    }

    async fn rollback(&self, savepoint_id: &str) -> Result<(), ApplianceError> {
        self.post(
            "core/firmware/revert",
            &serde_json::json!({"revision": savepoint_id}),
        )
        .await?;
        Ok(())
    }

    async fn list_firewall_rules(&self) -> Result<Vec<FirewallRule>, ApplianceError> {
        let data = self.get("firewall/filter/searchRule").await?;
        let mut rules = Vec::new();

        if let Some(rows) = data["rows"].as_array() {
            for row in rows {
                rules.push(FirewallRule {
                    id: row["uuid"].as_str().unwrap_or("").to_string(),
                    action: match row["action"].as_str() {
                        Some("pass") => crate::FirewallAction::Allow,
                        Some("reject") => crate::FirewallAction::Reject,
                        _ => crate::FirewallAction::Deny,
                    },
                    src: row["source_net"].as_str().unwrap_or("any").to_string(),
                    dst: row["destination_net"].as_str().unwrap_or("any").to_string(),
                    port: row["destination_port"]
                        .as_str()
                        .unwrap_or("any")
                        .to_string(),
                    protocol: row["protocol"].as_str().unwrap_or("any").to_string(),
                    enabled: row["enabled"].as_str() == Some("1"),
                    description: row["description"].as_str().unwrap_or("").to_string(),
                });
            }
        }

        Ok(rules)
    }

    async fn add_firewall_rule(&self, rule: &FirewallRule) -> Result<String, ApplianceError> {
        let body = serde_json::json!({
            "rule": {
                "action": match rule.action {
                    crate::FirewallAction::Allow => "pass",
                    crate::FirewallAction::Deny => "block",
                    crate::FirewallAction::Reject => "reject",
                },
                "source_net": rule.src,
                "destination_net": rule.dst,
                "destination_port": rule.port,
                "protocol": rule.protocol,
                "enabled": if rule.enabled { "1" } else { "0" },
                "description": rule.description,
            }
        });
        let result = self.post("firewall/filter/addRule", &body).await?;
        let uuid = result["uuid"].as_str().unwrap_or("").to_string();
        // Apply changes
        let _ = self
            .post("firewall/filter/apply", &serde_json::json!({}))
            .await;
        Ok(uuid)
    }

    async fn delete_firewall_rule(&self, rule_id: &str) -> Result<(), ApplianceError> {
        self.post(
            &format!("firewall/filter/delRule/{rule_id}"),
            &serde_json::json!({}),
        )
        .await?;
        let _ = self
            .post("firewall/filter/apply", &serde_json::json!({}))
            .await;
        Ok(())
    }

    async fn get_health(&self) -> Result<HealthStatus, ApplianceError> {
        let data = self.get("diagnostics/system/systemResources").await?;
        Ok(HealthStatus {
            cpu_percent: data["cpu"]["used"].as_str().and_then(|s| s.parse().ok()),
            memory_percent: data["memory"]["used"].as_str().and_then(|s| s.parse().ok()),
            disk_percent: data["disk"]["used"].as_str().and_then(|s| s.parse().ok()),
            temperature_celsius: None,
            alerts: vec![],
        })
    }

    async fn reboot(&self) -> Result<(), ApplianceError> {
        self.post("core/system/reboot", &serde_json::json!({}))
            .await?;
        Ok(())
    }

    async fn check_firmware_update(&self) -> Result<Option<String>, ApplianceError> {
        let data = self.get("core/firmware/status").await?;
        if data["status"] == "update" {
            Ok(Some(
                data["product_version"]
                    .as_str()
                    .unwrap_or("available")
                    .to_string(),
            ))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_with_correct_credentials() {
        let creds = Credentials::ApiKeySecret {
            key: "test-key".into(),
            secret: "test-secret".into(),
        };
        let driver = OPNsenseDriver::new("192.168.1.1", &creds).unwrap();
        assert_eq!(driver.driver_name(), "opnsense");
        assert_eq!(driver.base_url, "https://192.168.1.1/api");
    }

    #[test]
    fn rejects_wrong_credential_type() {
        let creds = Credentials::BearerToken {
            token: "tok".into(),
        };
        assert!(OPNsenseDriver::new("192.168.1.1", &creds).is_err());
    }
}
