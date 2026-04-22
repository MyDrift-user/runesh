//! UniFi Network Controller reference driver.
//!
//! Targets Ubiquiti's controller REST API. The concrete URL shape differs
//! between the "classic" standalone controller and a UniFi OS appliance
//! (UDM / UDR / UniFi OS Server): standalone exposes `/api/...` directly,
//! while UniFi OS multiplexes everything under `/proxy/network/...`. Both
//! shapes are supported by supplying the right `base_path` at
//! [`UniFiDriver::connect`] time.
//!
//! Auth model:
//! - `POST {base}/login` with `{ username, password }` sets a session
//!   cookie named `TOKEN` (UniFi OS) or `unifises` (classic). The login
//!   response also returns an `X-CSRF-Token` header that must be echoed
//!   on every subsequent mutating request.
//! - The cookie is refreshed on every request by the server, so we
//!   re-authenticate only when the controller answers 401.
//!
//! Scope of this first reference implementation:
//! - `get_identity` via `{base}/self` plus `{base}/s/{site}/stat/sysinfo`
//! - `list_firewall_rules`, `add_firewall_rule`, `delete_firewall_rule`
//!   via `{base}/s/{site}/rest/firewallrule`
//! - `get_health` via `{base}/s/{site}/stat/health`
//!
//! Methods not covered here (`list_interfaces`, `apply_config`,
//! `create_savepoint`, `rollback`, `reboot`, `check_firmware_update`) fall
//! through to the trait defaults or return `NotSupported`. The consumer
//! that needs them can shell out to the UniFi CLI or add the specific
//! endpoint on top of the session helpers exposed by this driver.

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    ApplianceDriver, ApplianceError, ConfigSavepoint, Credentials, DeviceIdentity, FirewallAction,
    FirewallRule, HealthStatus, NetInterface,
};

/// UniFi Network Controller driver.
pub struct UniFiDriver {
    client: Client,
    base_url: String,
    /// Prefix before the API namespace. `"/api"` for classic controllers,
    /// `"/proxy/network/api"` for UniFi OS.
    base_path: String,
    /// UniFi site id. `"default"` is the out-of-box site.
    site: String,
    username: String,
    password: SecretString,
    /// Cached CSRF token returned by the most recent login. Must be sent
    /// as `X-CSRF-Token` on every mutating request.
    session: Arc<Mutex<Option<SessionState>>>,
    /// Whether to verify the controller's TLS certificate.
    ///
    /// UniFi controllers ship with a self-signed cert out of the box.
    /// Default is `true`; callers running against stock certs must
    /// explicitly opt into insecure mode with [`Self::insecure_tls`].
    insecure_tls: bool,
}

#[derive(Clone)]
struct SessionState {
    csrf: String,
}

impl UniFiDriver {
    /// Build a driver for a classic standalone controller (API at `/api`).
    pub fn classic(host: &str, site: &str, credentials: Credentials) -> Result<Self, ApplianceError> {
        Self::build(host, "/api", site, credentials, false)
    }

    /// Build a driver for a UniFi OS appliance (API at `/proxy/network/api`).
    pub fn unifi_os(host: &str, site: &str, credentials: Credentials) -> Result<Self, ApplianceError> {
        Self::build(host, "/proxy/network/api", site, credentials, false)
    }

    /// Opt into insecure TLS (accept invalid certs). Needed against a
    /// stock controller with its self-signed certificate. Never enable
    /// this against a controller exposed to untrusted networks.
    pub fn insecure_tls(mut self) -> Self {
        self.insecure_tls = true;
        self
    }

    fn build(
        host: &str,
        base_path: &str,
        site: &str,
        credentials: Credentials,
        insecure_tls: bool,
    ) -> Result<Self, ApplianceError> {
        let (username, password) = match credentials {
            Credentials::UsernamePassword { username, password } => (username, password),
            _ => {
                return Err(ApplianceError::NotSupported(
                    "UniFi driver accepts only Credentials::UsernamePassword".into(),
                ));
            }
        };

        // `cookie_store = true` lets reqwest manage the session cookie
        // (`TOKEN` on UniFi OS, `unifises` on classic) without the caller
        // having to track it by hand.
        let client = Client::builder()
            .danger_accept_invalid_certs(insecure_tls)
            .cookie_store(true)
            .build()
            .map_err(|e| ApplianceError::ConnectionFailed(e.to_string()))?;

        let base_url = if host.starts_with("http://") || host.starts_with("https://") {
            host.trim_end_matches('/').to_string()
        } else {
            format!("https://{}", host.trim_end_matches('/'))
        };

        Ok(Self {
            client,
            base_url,
            base_path: base_path.to_string(),
            site: site.to_string(),
            username,
            password,
            session: Arc::new(Mutex::new(None)),
            insecure_tls,
        })
    }

    fn api_url(&self, tail: &str) -> String {
        format!("{}{}{tail}", self.base_url, self.base_path)
    }

    fn site_url(&self, tail: &str) -> String {
        format!(
            "{}{}/s/{}{tail}",
            self.base_url, self.base_path, self.site
        )
    }

    /// Log in and cache the CSRF token. Always re-authenticates.
    async fn login(&self) -> Result<SessionState, ApplianceError> {
        #[derive(Serialize)]
        struct LoginBody<'a> {
            username: &'a str,
            password: &'a str,
        }
        let resp = self
            .client
            .post(self.api_url("/auth/login"))
            .json(&LoginBody {
                username: &self.username,
                password: self.password.expose_secret(),
            })
            .send()
            .await
            .map_err(ApplianceError::Request)?;

        // Try the legacy /login too for classic controllers that never got
        // the /auth/login path.
        let resp = if resp.status() == StatusCode::NOT_FOUND {
            self.client
                .post(self.api_url("/login"))
                .json(&LoginBody {
                    username: &self.username,
                    password: self.password.expose_secret(),
                })
                .send()
                .await
                .map_err(ApplianceError::Request)?
        } else {
            resp
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(if status == StatusCode::UNAUTHORIZED {
                ApplianceError::AuthFailed(redact_body(&body))
            } else {
                ApplianceError::ApiError(format!("login {status}: {}", redact_body(&body)))
            });
        }

        let csrf = resp
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let state = SessionState { csrf };
        *self.session.lock().await = Some(state.clone());
        Ok(state)
    }

    async fn session_state(&self) -> Result<SessionState, ApplianceError> {
        {
            if let Some(s) = self.session.lock().await.clone() {
                return Ok(s);
            }
        }
        self.login().await
    }

    /// Apply the cached CSRF header to a request builder.
    async fn with_csrf(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ApplianceError> {
        let state = self.session_state().await?;
        Ok(if state.csrf.is_empty() {
            req
        } else {
            req.header("X-CSRF-Token", state.csrf)
        })
    }

    /// Send a request, transparently re-authenticating once on 401.
    async fn send_with_retry<F>(&self, build: F) -> Result<reqwest::Response, ApplianceError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let first = self
            .with_csrf(build())
            .await?
            .send()
            .await
            .map_err(ApplianceError::Request)?;
        if first.status() == StatusCode::UNAUTHORIZED {
            *self.session.lock().await = None;
            let _ = self.login().await?;
            return self
                .with_csrf(build())
                .await?
                .send()
                .await
                .map_err(ApplianceError::Request);
        }
        Ok(first)
    }

    /// Parse a standard UniFi envelope `{ "meta": { "rc": "ok" }, "data": [...] }`.
    async fn parse_data<T: for<'de> Deserialize<'de>>(
        resp: reqwest::Response,
    ) -> Result<Vec<T>, ApplianceError> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ApplianceError::ApiError(format!(
                "{status}: {}",
                redact_body(&text)
            )));
        }
        let env: Envelope<T> = serde_json::from_str(&text)
            .map_err(|e| ApplianceError::ParseError(format!("{e}; body={}", redact_body(&text))))?;
        if env.meta.rc != "ok" {
            return Err(ApplianceError::ApiError(format!(
                "controller returned rc={}",
                env.meta.rc
            )));
        }
        Ok(env.data)
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    meta: Meta,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
struct Meta {
    rc: String,
}

#[derive(Deserialize)]
struct UnifiSysinfo {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize)]
struct UnifiHealth {
    #[serde(default)]
    subsystem: String,
    #[serde(default)]
    status: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct UnifiFirewallRule {
    #[serde(rename = "_id", default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_ruleset")]
    ruleset: String,
    #[serde(default)]
    rule_index: u32,
    #[serde(default = "default_true_bool")]
    enabled: bool,
    #[serde(default = "default_action")]
    action: String,
    #[serde(default = "default_protocol")]
    protocol: String,
    #[serde(default)]
    src_firewallgroup_ids: Vec<String>,
    #[serde(default)]
    dst_firewallgroup_ids: Vec<String>,
    #[serde(default)]
    src_address: String,
    #[serde(default)]
    dst_address: String,
    #[serde(default)]
    dst_port: String,
}

fn default_ruleset() -> String {
    "WAN_IN".into()
}
fn default_true_bool() -> bool {
    true
}
fn default_action() -> String {
    "accept".into()
}
fn default_protocol() -> String {
    "all".into()
}

fn redact_body(body: &str) -> String {
    // UniFi login errors sometimes echo the submitted username but never
    // the password. Strip any `"password"` substring just in case.
    let mut out = body.to_string();
    if let Some(idx) = out.to_lowercase().find("\"password\"") {
        out.truncate(idx);
        out.push_str("<...redacted...>");
    }
    out
}

fn map_action(s: &str) -> FirewallAction {
    match s.to_ascii_lowercase().as_str() {
        "accept" | "allow" => FirewallAction::Allow,
        "reject" => FirewallAction::Reject,
        _ => FirewallAction::Deny,
    }
}

fn map_action_to_unifi(a: FirewallAction) -> &'static str {
    match a {
        FirewallAction::Allow => "accept",
        FirewallAction::Deny => "drop",
        FirewallAction::Reject => "reject",
    }
}

#[async_trait]
impl ApplianceDriver for UniFiDriver {
    fn driver_name(&self) -> &str {
        "unifi"
    }

    async fn get_identity(&self) -> Result<DeviceIdentity, ApplianceError> {
        let url = self.site_url("/stat/sysinfo");
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let rows: Vec<UnifiSysinfo> = Self::parse_data(resp).await?;
        let sys = rows.into_iter().next().unwrap_or(UnifiSysinfo {
            hostname: String::new(),
            version: String::new(),
        });
        Ok(DeviceIdentity {
            hostname: sys.hostname,
            model: String::new(),
            serial: String::new(),
            firmware_version: sys.version,
            vendor: "Ubiquiti".into(),
            uptime_secs: None,
        })
    }

    async fn list_interfaces(&self) -> Result<Vec<NetInterface>, ApplianceError> {
        // Interface enumeration on UniFi is per-device and uses a
        // different endpoint shape per model. Leave unsupported for this
        // reference driver; consumers that need it can query
        // /stat/device and iterate themselves.
        Err(ApplianceError::NotSupported(
            "list_interfaces: query /stat/device per site, then per-device /stat/port".into(),
        ))
    }

    async fn get_config(&self) -> Result<serde_json::Value, ApplianceError> {
        // UniFi exposes config fragments via /rest/setting/<key>. A
        // monolithic "running config" does not exist in the API.
        Err(ApplianceError::NotSupported(
            "get_config: UniFi config is fragmented under /rest/setting/<key>".into(),
        ))
    }

    async fn create_savepoint(
        &self,
        _description: &str,
    ) -> Result<ConfigSavepoint, ApplianceError> {
        Err(ApplianceError::NotSupported(
            "UniFi does not expose a savepoint API; use controller backups".into(),
        ))
    }

    async fn rollback(&self, _savepoint_id: &str) -> Result<(), ApplianceError> {
        Err(ApplianceError::NotSupported(
            "UniFi does not expose a savepoint API".into(),
        ))
    }

    async fn list_firewall_rules(&self) -> Result<Vec<FirewallRule>, ApplianceError> {
        let url = self.site_url("/rest/firewallrule");
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let rows: Vec<UnifiFirewallRule> = Self::parse_data(resp).await?;
        Ok(rows
            .into_iter()
            .map(|r| FirewallRule {
                id: r.id,
                action: map_action(&r.action),
                src: if r.src_address.is_empty() {
                    r.src_firewallgroup_ids.join(",")
                } else {
                    r.src_address
                },
                dst: if r.dst_address.is_empty() {
                    r.dst_firewallgroup_ids.join(",")
                } else {
                    r.dst_address
                },
                port: r.dst_port,
                protocol: r.protocol,
                enabled: r.enabled,
                description: r.name,
            })
            .collect())
    }

    async fn add_firewall_rule(&self, rule: &FirewallRule) -> Result<String, ApplianceError> {
        let url = self.site_url("/rest/firewallrule");
        let body = serde_json::json!({
            "name": rule.description,
            "enabled": rule.enabled,
            "action": map_action_to_unifi(rule.action),
            "protocol": if rule.protocol.is_empty() { "all" } else { rule.protocol.as_str() },
            "ruleset": "WAN_IN",
            "rule_index": 2000,
            "src_address": rule.src,
            "dst_address": rule.dst,
            "dst_port": rule.port,
        });
        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;
        let rows: Vec<UnifiFirewallRule> = Self::parse_data(resp).await?;
        rows.into_iter()
            .next()
            .map(|r| r.id)
            .ok_or_else(|| ApplianceError::ApiError("add_firewall_rule: empty data".into()))
    }

    async fn delete_firewall_rule(&self, rule_id: &str) -> Result<(), ApplianceError> {
        if rule_id.is_empty()
            || !rule_id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(ApplianceError::ApiError(format!(
                "invalid firewall rule id: {rule_id}"
            )));
        }
        let url = self.site_url(&format!("/rest/firewallrule/{rule_id}"));
        let resp = self.send_with_retry(|| self.client.delete(&url)).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApplianceError::ApiError(format!(
                "delete_firewall_rule {status}: {}",
                redact_body(&body)
            )));
        }
        Ok(())
    }

    async fn get_health(&self) -> Result<HealthStatus, ApplianceError> {
        let url = self.site_url("/stat/health");
        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let rows: Vec<UnifiHealth> = Self::parse_data(resp).await?;
        let alerts: Vec<String> = rows
            .into_iter()
            .filter(|h| h.status.eq_ignore_ascii_case("warning") || h.status.eq_ignore_ascii_case("error"))
            .map(|h| format!("{}: {}", h.subsystem, h.status))
            .collect();
        Ok(HealthStatus {
            cpu_percent: None,
            memory_percent: None,
            disk_percent: None,
            temperature_celsius: None,
            alerts,
        })
    }

    async fn reboot(&self) -> Result<(), ApplianceError> {
        Err(ApplianceError::NotSupported(
            "UniFi reboot is per-device via /cmd/devmgr {cmd:restart, mac:...}".into(),
        ))
    }

    async fn check_firmware_update(&self) -> Result<Option<String>, ApplianceError> {
        Err(ApplianceError::NotSupported(
            "check_firmware_update: UniFi firmware is per-device".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials::UsernamePassword {
            username: "admin".into(),
            password: SecretString::from("hunter2".to_string()),
        }
    }

    #[test]
    fn classic_constructor_builds_urls() {
        let d = UniFiDriver::classic("ctrl.example.com", "default", creds()).unwrap();
        assert_eq!(d.api_url("/login"), "https://ctrl.example.com/api/login");
        assert_eq!(
            d.site_url("/stat/health"),
            "https://ctrl.example.com/api/s/default/stat/health"
        );
    }

    #[test]
    fn unifi_os_constructor_builds_proxy_urls() {
        let d = UniFiDriver::unifi_os("udm.example.com", "default", creds()).unwrap();
        assert_eq!(
            d.api_url("/login"),
            "https://udm.example.com/proxy/network/api/login"
        );
        assert_eq!(
            d.site_url("/rest/firewallrule"),
            "https://udm.example.com/proxy/network/api/s/default/rest/firewallrule"
        );
    }

    #[test]
    fn constructor_rejects_non_username_password_credentials() {
        let result = UniFiDriver::classic(
            "x",
            "default",
            Credentials::BearerToken {
                token: SecretString::from("t".to_string()),
            },
        );
        assert!(result.is_err());
        match result.err().unwrap() {
            ApplianceError::NotSupported(_) => {}
            e => panic!("expected NotSupported, got {e:?}"),
        }
    }

    #[test]
    fn accepts_scheme_already_in_host() {
        let d = UniFiDriver::classic("http://ctrl:8443", "default", creds()).unwrap();
        assert_eq!(d.api_url("/login"), "http://ctrl:8443/api/login");
    }

    #[test]
    fn redact_body_strips_password_tail() {
        let r = redact_body(r#"{"error":"bad","password":"hunter2"}"#);
        assert!(!r.contains("hunter2"));
        assert!(r.contains("<...redacted...>"));
    }

    #[test]
    fn firewall_action_roundtrip() {
        assert_eq!(map_action("accept"), FirewallAction::Allow);
        assert_eq!(map_action("drop"), FirewallAction::Deny);
        assert_eq!(map_action("reject"), FirewallAction::Reject);
        assert_eq!(map_action("something-else"), FirewallAction::Deny);
        assert_eq!(map_action_to_unifi(FirewallAction::Allow), "accept");
        assert_eq!(map_action_to_unifi(FirewallAction::Deny), "drop");
        assert_eq!(map_action_to_unifi(FirewallAction::Reject), "reject");
    }
}
