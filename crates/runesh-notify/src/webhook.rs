//! Webhook notification channel.
//!
//! Sends notifications as JSON POST requests to arbitrary URLs.
//! Works for Slack incoming webhooks, Discord webhooks, Ntfy, Gotify,
//! and any custom HTTP endpoint.
//!
//! Security:
//! - Shared [`reqwest::Client`] with per-request timeout
//! - Optional URL allowlist, private/loopback/IMDS address blocking (SSRF)
//! - Optional HMAC-SHA256 body signing
//! - Retry with exponential backoff + full jitter, honouring Retry-After

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;

use crate::{Notification, NotificationChannel, SendResult};

/// `(body, headers)` returned from [`WebhookChannel::build_body`].
type BuiltBody = (String, Vec<(&'static str, String)>);

/// Payload format for the webhook.
pub enum WebhookFormat {
    /// Send the raw Notification struct as JSON.
    Raw,
    /// Slack-compatible format (`{"text": "..."}`).
    Slack,
    /// Discord-compatible format (`{"content": "..."}`).
    Discord,
    /// Ntfy-compatible format (POST body = message, headers for title/priority).
    Ntfy,
}

/// URL policy controlling which destinations a webhook can reach.
#[derive(Debug, Clone)]
pub struct WebhookUrlPolicy {
    /// If non-empty, host must match one of these suffixes (case-insensitive).
    /// Example: `"hooks.slack.com"` matches `https://hooks.slack.com/services/...`.
    pub allowlist: Vec<String>,
    /// Block loopback / link-local / private / IMDS addresses.
    pub block_private_ips: bool,
}

impl Default for WebhookUrlPolicy {
    fn default() -> Self {
        Self {
            allowlist: Vec::new(),
            block_private_ips: true,
        }
    }
}

/// Retry policy for transient webhook failures.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub retry_count: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            retry_count: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Webhook channel configuration.
pub struct WebhookChannel {
    pub name: String,
    pub url: String,
    pub auth_header: Option<String>,
    pub format: WebhookFormat,
    pub url_policy: WebhookUrlPolicy,
    pub retry: RetryPolicy,
    /// If set, bodies are signed as `X-Runesh-Signature: hex(hmac_sha256(secret, body))`.
    pub shared_secret: Option<String>,
}

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

impl WebhookChannel {
    /// Create a simple raw JSON webhook (SSRF protection on by default).
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            auth_header: None,
            format: WebhookFormat::Raw,
            url_policy: WebhookUrlPolicy::default(),
            retry: RetryPolicy::default(),
            shared_secret: None,
        }
    }

    pub fn slack(url: &str) -> Self {
        Self::new("slack", url).with_format(WebhookFormat::Slack)
    }

    pub fn discord(url: &str) -> Self {
        Self::new("discord", url).with_format(WebhookFormat::Discord)
    }

    pub fn ntfy(url: &str) -> Self {
        Self::new("ntfy", url).with_format(WebhookFormat::Ntfy)
    }

    pub fn with_format(mut self, format: WebhookFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_url_policy(mut self, policy: WebhookUrlPolicy) -> Self {
        self.url_policy = policy;
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_shared_secret(mut self, secret: impl Into<String>) -> Self {
        self.shared_secret = Some(secret.into());
        self
    }

    pub fn with_auth_header(mut self, header: impl Into<String>) -> Self {
        self.auth_header = Some(header.into());
        self
    }

    fn build_body(&self, notification: &Notification) -> Result<BuiltBody, String> {
        match self.format {
            WebhookFormat::Raw => {
                let body =
                    serde_json::to_string(notification).map_err(|e| format!("serialize: {e}"))?;
                Ok((body, vec![("Content-Type", "application/json".into())]))
            }
            WebhookFormat::Slack => {
                let body = serde_json::to_string(&serde_json::json!({
                    "text": format!(
                        "*[{}]* {}\n{}",
                        notification.severity_label(),
                        notification.title,
                        notification.body,
                    ),
                }))
                .map_err(|e| format!("serialize: {e}"))?;
                Ok((body, vec![("Content-Type", "application/json".into())]))
            }
            WebhookFormat::Discord => {
                let body = serde_json::to_string(&serde_json::json!({
                    "content": format!(
                        "**[{}]** {}\n{}",
                        notification.severity_label(),
                        notification.title,
                        notification.body,
                    ),
                }))
                .map_err(|e| format!("serialize: {e}"))?;
                Ok((body, vec![("Content-Type", "application/json".into())]))
            }
            WebhookFormat::Ntfy => {
                let priority = match notification.severity {
                    crate::NotifySeverity::Critical => "5",
                    crate::NotifySeverity::Warning => "3",
                    _ => "2",
                };
                Ok((
                    notification.body.clone(),
                    vec![
                        ("Title", notification.title.clone()),
                        ("Priority", priority.into()),
                        ("Content-Type", "text/plain".into()),
                    ],
                ))
            }
        }
    }
}

#[async_trait]
impl NotificationChannel for WebhookChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, notification: &Notification) -> SendResult {
        // Validate URL and enforce policy.
        if let Err(e) = validate_url(&self.url, &self.url_policy).await {
            return SendResult {
                success: false,
                channel: self.name.clone(),
                error: Some(format!("url rejected: {e}")),
            };
        }

        let (body, headers) = match self.build_body(notification) {
            Ok(x) => x,
            Err(e) => {
                return SendResult {
                    success: false,
                    channel: self.name.clone(),
                    error: Some(e),
                };
            }
        };

        match send_with_retry(
            shared_client(),
            &self.url,
            body.into_bytes(),
            headers,
            self.auth_header.as_deref(),
            self.shared_secret.as_deref(),
            &self.retry,
        )
        .await
        {
            Ok(()) => SendResult {
                success: true,
                channel: self.name.clone(),
                error: None,
            },
            Err(e) => SendResult {
                success: false,
                channel: self.name.clone(),
                error: Some(e),
            },
        }
    }
}

/// Classify an IP address as unsafe for outbound SSRF.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // Loopback 127.0.0.0/8, private ranges, link-local, IMDS, reserved.
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // AWS/GCP/Azure IMDS
                || v4.octets() == [169, 254, 169, 254]
                // Shared address space (CGNAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                // TEST-NET / benchmarking reserved
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                // Unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped loopback
                || v6.to_ipv4_mapped().map(|v4| is_blocked_ip(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

async fn validate_url(url: &str, policy: &WebhookUrlPolicy) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("parse: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("unsupported scheme {s}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "missing host".to_string())?;

    if !policy.allowlist.is_empty() {
        let lowered = host.to_ascii_lowercase();
        let allowed = policy.allowlist.iter().any(|suffix| {
            lowered == suffix.to_ascii_lowercase()
                || lowered.ends_with(&format!(".{}", suffix.to_ascii_lowercase()))
        });
        if !allowed {
            return Err(format!("host {host} not in allowlist"));
        }
    }

    if policy.block_private_ips {
        // Resolve all addresses and reject if ANY are private/loopback/IMDS.
        let port = parsed.port_or_known_default().unwrap_or(80);
        let host_for_dns = host.to_string();
        let addrs = tokio::task::spawn_blocking(move || {
            use std::net::ToSocketAddrs;
            (host_for_dns.as_str(), port)
                .to_socket_addrs()
                .map(|it| it.collect::<Vec<_>>())
        })
        .await
        .map_err(|e| format!("dns task: {e}"))?
        .map_err(|e| format!("dns: {e}"))?;
        for sa in addrs {
            if is_blocked_ip(sa.ip()) {
                return Err(format!("address {} is private/loopback/IMDS", sa.ip()));
            }
        }
    }
    Ok(())
}

async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    extra_headers: Vec<(&'static str, String)>,
    auth: Option<&str>,
    shared_secret: Option<&str>,
    policy: &RetryPolicy,
) -> Result<(), String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let body = Arc::new(body);

    let signature = if let Some(secret) = shared_secret {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
            .map_err(|e| format!("hmac init: {e}"))?;
        mac.update(&body);
        Some(hex::encode(mac.finalize().into_bytes()))
    } else {
        None
    };

    let mut attempt: u32 = 0;
    let mut last_err: String;
    loop {
        attempt += 1;
        let mut req = client.post(url).body((*body).clone());
        for (k, v) in &extra_headers {
            req = req.header(*k, v);
        }
        if let Some(a) = auth {
            req = req.header("Authorization", a);
        }
        if let Some(sig) = &signature {
            req = req.header("X-Runesh-Signature", sig);
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                // Respect Retry-After on 429 / 503
                let retry_after = if status.as_u16() == 429 || status.as_u16() == 503 {
                    resp.headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(Duration::from_secs)
                } else {
                    None
                };
                last_err = format!("HTTP {status}");
                if attempt > policy.retry_count {
                    return Err(last_err);
                }
                // Only retry 5xx and 429, not other 4xx
                if !(status.is_server_error() || status.as_u16() == 429) {
                    return Err(last_err);
                }
                let delay = match retry_after {
                    Some(d) => d.min(policy.max_delay),
                    None => backoff_delay(attempt, policy),
                };
                tokio::time::sleep(delay).await;
            }
            Err(e) => {
                last_err = e.to_string();
                if attempt > policy.retry_count {
                    return Err(last_err);
                }
                tokio::time::sleep(backoff_delay(attempt, policy)).await;
            }
        }
    }
}

fn backoff_delay(attempt: u32, policy: &RetryPolicy) -> Duration {
    use rand::Rng;
    let exp = 2u64.saturating_pow(attempt.saturating_sub(1));
    let max_nanos = policy
        .base_delay
        .saturating_mul(exp as u32)
        .min(policy.max_delay)
        .as_nanos() as u64;
    if max_nanos == 0 {
        return Duration::from_millis(0);
    }
    let jitter_nanos = rand::thread_rng().gen_range(0..=max_nanos);
    Duration::from_nanos(jitter_nanos)
}

impl Notification {
    pub(crate) fn severity_label(&self) -> &str {
        match self.severity {
            crate::NotifySeverity::Info => "INFO",
            crate::NotifySeverity::Warning => "WARN",
            crate::NotifySeverity::Critical => "CRIT",
            crate::NotifySeverity::Resolved => "OK",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn blocks_imds() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        // Public addresses should pass (except we whitelist nothing here).
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[tokio::test]
    async fn validate_url_rejects_loopback_by_default() {
        let policy = WebhookUrlPolicy::default();
        let err = validate_url("http://127.0.0.1:12345/hook", &policy)
            .await
            .unwrap_err();
        assert!(
            err.contains("private") || err.contains("loopback") || err.contains("IMDS"),
            "err={err}"
        );
    }

    #[tokio::test]
    async fn validate_url_allowlist_enforced() {
        let policy = WebhookUrlPolicy {
            allowlist: vec!["example.com".into()],
            block_private_ips: false,
        };
        assert!(validate_url("http://example.com/a", &policy).await.is_ok());
        assert!(
            validate_url("http://hooks.slack.com/b", &policy)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn webhook_rejects_imds() {
        let ch = WebhookChannel::new("imds", "http://169.254.169.254/latest/meta-data/");
        let notif = crate::Notification {
            severity: crate::NotifySeverity::Critical,
            title: "t".into(),
            body: "b".into(),
            source: None,
            url: None,
            fields: Default::default(),
        };
        let result = ch.send(&notif).await;
        assert!(!result.success);
        let msg = result.error.unwrap();
        assert!(
            msg.to_lowercase().contains("imds") || msg.to_lowercase().contains("private"),
            "msg={msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn webhook_retries_respect_retry_after() {
        // Spin up a tiny server that returns 503 + Retry-After: 1 once, then 200.
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let calls_srv = calls.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let calls2 = calls_srv.clone();
                tokio::spawn(async move {
                    // Read request headers up to \r\n\r\n.
                    let mut buf = [0u8; 4096];
                    let mut total = Vec::new();
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                total.extend_from_slice(&buf[..n]);
                                if total.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    let count = calls2.fetch_add(1, Ordering::SeqCst);
                    if count == 0 {
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    } else {
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    }
                });
            }
        });

        let url = format!("http://127.0.0.1:{}/", addr.port());
        let ch = WebhookChannel::new("test", &url)
            .with_url_policy(WebhookUrlPolicy {
                allowlist: vec![],
                block_private_ips: false,
            })
            .with_retry(RetryPolicy {
                retry_count: 2,
                base_delay: Duration::from_millis(50),
                max_delay: Duration::from_secs(5),
            });
        let notif = crate::Notification {
            severity: crate::NotifySeverity::Critical,
            title: "t".into(),
            body: "b".into(),
            source: None,
            url: None,
            fields: Default::default(),
        };
        let start = std::time::Instant::now();
        let result = ch.send(&notif).await;
        let elapsed = start.elapsed();
        assert!(result.success, "err={:?}", result.error);
        // Second call happened after >= 1s due to Retry-After: 1.
        assert!(elapsed >= Duration::from_millis(900), "elapsed={elapsed:?}");
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn webhook_hmac_signature_emitted() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let c2 = captured.clone();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let mut total = Vec::new();
            loop {
                match sock.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        total.extend_from_slice(&buf[..n]);
                        if total.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => return,
                }
            }
            *c2.lock().unwrap() = total;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        });

        let url = format!("http://127.0.0.1:{}/", addr.port());
        let ch = WebhookChannel::new("test", &url)
            .with_url_policy(WebhookUrlPolicy {
                allowlist: vec![],
                block_private_ips: false,
            })
            .with_shared_secret("hunter2")
            .with_retry(RetryPolicy {
                retry_count: 0,
                base_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(10),
            });
        let notif = crate::Notification {
            severity: crate::NotifySeverity::Critical,
            title: "t".into(),
            body: "b".into(),
            source: None,
            url: None,
            fields: Default::default(),
        };
        let result = ch.send(&notif).await;
        assert!(result.success, "err={:?}", result.error);
        let req = String::from_utf8_lossy(&captured.lock().unwrap().clone()).to_string();
        assert!(
            req.to_lowercase().contains("x-runesh-signature:"),
            "expected X-Runesh-Signature header: {req}"
        );
    }
}
