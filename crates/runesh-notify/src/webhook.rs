//! Webhook notification channel.
//!
//! Sends notifications as JSON POST requests to arbitrary URLs.
//! Works for Slack incoming webhooks, Discord webhooks, Ntfy, Gotify,
//! and any custom HTTP endpoint.

use async_trait::async_trait;

use crate::{Notification, NotificationChannel, SendResult};

/// Webhook channel configuration.
pub struct WebhookChannel {
    /// Channel name for logging.
    pub name: String,
    /// Webhook URL.
    pub url: String,
    /// Optional authorization header value.
    pub auth_header: Option<String>,
    /// Format the payload as (default: raw notification JSON).
    pub format: WebhookFormat,
}

/// Payload format for the webhook.
pub enum WebhookFormat {
    /// Send the raw Notification struct as JSON.
    Raw,
    /// Slack-compatible format ({"text": "..."}).
    Slack,
    /// Discord-compatible format ({"content": "..."}).
    Discord,
    /// Ntfy-compatible format (POST body = message, headers for title/priority).
    Ntfy,
}

impl WebhookChannel {
    /// Create a simple raw JSON webhook.
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
            auth_header: None,
            format: WebhookFormat::Raw,
        }
    }

    /// Create a Slack incoming webhook.
    pub fn slack(url: &str) -> Self {
        Self {
            name: "slack".to_string(),
            url: url.to_string(),
            auth_header: None,
            format: WebhookFormat::Slack,
        }
    }

    /// Create a Discord webhook.
    pub fn discord(url: &str) -> Self {
        Self {
            name: "discord".to_string(),
            url: url.to_string(),
            auth_header: None,
            format: WebhookFormat::Discord,
        }
    }

    /// Create an Ntfy channel.
    pub fn ntfy(url: &str) -> Self {
        Self {
            name: "ntfy".to_string(),
            url: url.to_string(),
            auth_header: None,
            format: WebhookFormat::Ntfy,
        }
    }
}

#[async_trait]
impl NotificationChannel for WebhookChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, notification: &Notification) -> SendResult {
        let client = reqwest::Client::new();

        let result = match &self.format {
            WebhookFormat::Raw => {
                let mut req = client.post(&self.url).json(notification);
                if let Some(auth) = &self.auth_header {
                    req = req.header("Authorization", auth);
                }
                req.send().await
            }
            WebhookFormat::Slack => {
                let body = serde_json::json!({
                    "text": format!("*[{}]* {}\n{}", notification.severity_label(), notification.title, notification.body),
                });
                client.post(&self.url).json(&body).send().await
            }
            WebhookFormat::Discord => {
                let body = serde_json::json!({
                    "content": format!("**[{}]** {}\n{}", notification.severity_label(), notification.title, notification.body),
                });
                client.post(&self.url).json(&body).send().await
            }
            WebhookFormat::Ntfy => {
                let priority = match notification.severity {
                    crate::NotifySeverity::Critical => "5",
                    crate::NotifySeverity::Warning => "3",
                    _ => "2",
                };
                client
                    .post(&self.url)
                    .header("Title", &notification.title)
                    .header("Priority", priority)
                    .body(notification.body.clone())
                    .send()
                    .await
            }
        };

        match result {
            Ok(resp) if resp.status().is_success() => SendResult {
                success: true,
                channel: self.name.clone(),
                error: None,
            },
            Ok(resp) => SendResult {
                success: false,
                channel: self.name.clone(),
                error: Some(format!("HTTP {}", resp.status())),
            },
            Err(e) => SendResult {
                success: false,
                channel: self.name.clone(),
                error: Some(e.to_string()),
            },
        }
    }
}

impl Notification {
    fn severity_label(&self) -> &str {
        match self.severity {
            crate::NotifySeverity::Info => "INFO",
            crate::NotifySeverity::Warning => "WARN",
            crate::NotifySeverity::Critical => "CRIT",
            crate::NotifySeverity::Resolved => "OK",
        }
    }
}
