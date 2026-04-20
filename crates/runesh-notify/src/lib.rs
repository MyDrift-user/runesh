#![deny(unsafe_code)]
//! Notification dispatch with pluggable channels.
//!
//! Each channel implements the `NotificationChannel` trait.
//! Notifications carry a severity, title, body, and optional fields.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[cfg(feature = "email")]
pub mod email;
pub mod webhook;

/// A notification to be sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Severity level.
    pub severity: NotifySeverity,
    /// Short title/subject.
    pub title: String,
    /// Message body (plain text or markdown).
    pub body: String,
    /// Source of the notification (check name, system, etc.).
    #[serde(default)]
    pub source: Option<String>,
    /// URL for more details.
    #[serde(default)]
    pub url: Option<String>,
    /// Additional key-value fields.
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
}

/// Notification severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifySeverity {
    Info,
    Warning,
    Critical,
    Resolved,
}

/// Result of sending a notification.
#[derive(Debug)]
pub struct SendResult {
    pub success: bool,
    pub channel: String,
    pub error: Option<String>,
}

/// Trait for notification channels.
#[async_trait]
pub trait NotificationChannel: Send + Sync {
    /// Channel name (for logging).
    fn name(&self) -> &str;

    /// Send a notification through this channel.
    async fn send(&self, notification: &Notification) -> SendResult;
}

/// Dispatches notifications to multiple channels.
pub struct Dispatcher {
    channels: Vec<Box<dyn NotificationChannel>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
        }
    }

    /// Add a channel to the dispatcher.
    pub fn add_channel(&mut self, channel: Box<dyn NotificationChannel>) {
        self.channels.push(channel);
    }

    /// Send a notification to all channels.
    /// Returns results for each channel.
    pub async fn send(&self, notification: &Notification) -> Vec<SendResult> {
        let mut results = Vec::new();
        for channel in &self.channels {
            let result = channel.send(notification).await;
            if !result.success {
                tracing::warn!(
                    channel = channel.name(),
                    error = ?result.error,
                    "notification send failed"
                );
            }
            results.push(result);
        }
        results
    }

    /// Send to channels matching a severity filter.
    pub async fn send_filtered(
        &self,
        notification: &Notification,
        min_severity: NotifySeverity,
    ) -> Vec<SendResult> {
        let dominated = matches!(
            (notification.severity, min_severity),
            (
                NotifySeverity::Info,
                NotifySeverity::Warning | NotifySeverity::Critical
            ) | (NotifySeverity::Warning, NotifySeverity::Critical)
        );
        if dominated {
            return Vec::new();
        }
        self.send(notification).await
    }

    /// Number of registered channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockChannel {
        name: String,
        should_fail: bool,
    }

    #[async_trait]
    impl NotificationChannel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }
        async fn send(&self, _notification: &Notification) -> SendResult {
            if self.should_fail {
                SendResult {
                    success: false,
                    channel: self.name.clone(),
                    error: Some("mock failure".into()),
                }
            } else {
                SendResult {
                    success: true,
                    channel: self.name.clone(),
                    error: None,
                }
            }
        }
    }

    fn test_notification() -> Notification {
        Notification {
            severity: NotifySeverity::Critical,
            title: "Server down".into(),
            body: "web-01 is not responding".into(),
            source: Some("http-check".into()),
            url: None,
            fields: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn dispatch_to_multiple_channels() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.add_channel(Box::new(MockChannel {
            name: "slack".into(),
            should_fail: false,
        }));
        dispatcher.add_channel(Box::new(MockChannel {
            name: "email".into(),
            should_fail: false,
        }));

        let results = dispatcher.send(&test_notification()).await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
    }

    #[tokio::test]
    async fn partial_failure() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.add_channel(Box::new(MockChannel {
            name: "ok".into(),
            should_fail: false,
        }));
        dispatcher.add_channel(Box::new(MockChannel {
            name: "broken".into(),
            should_fail: true,
        }));

        let results = dispatcher.send(&test_notification()).await;
        assert!(results[0].success);
        assert!(!results[1].success);
    }

    #[tokio::test]
    async fn severity_filter() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.add_channel(Box::new(MockChannel {
            name: "ch".into(),
            should_fail: false,
        }));

        // Info notification filtered when min is Critical
        let mut notif = test_notification();
        notif.severity = NotifySeverity::Info;
        let results = dispatcher
            .send_filtered(&notif, NotifySeverity::Critical)
            .await;
        assert!(results.is_empty());

        // Critical passes the filter
        notif.severity = NotifySeverity::Critical;
        let results = dispatcher
            .send_filtered(&notif, NotifySeverity::Critical)
            .await;
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn notification_serializes() {
        let n = test_notification();
        let json = serde_json::to_string(&n).unwrap();
        let parsed: Notification = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "Server down");
    }
}
