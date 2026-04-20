//! Email notification channel via SMTP (requires `email` feature).

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::{Notification, NotificationChannel, SendResult};

/// SMTP email channel.
pub struct EmailChannel {
    pub name: String,
    pub from: String,
    pub to: Vec<String>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
}

#[async_trait]
impl NotificationChannel for EmailChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, notification: &Notification) -> SendResult {
        let subject = format!("[{}] {}", notification.severity_label(), notification.title);

        for recipient in &self.to {
            let email = match Message::builder()
                .from(self.from.parse().unwrap())
                .to(recipient.parse().unwrap())
                .subject(&subject)
                .header(ContentType::TEXT_PLAIN)
                .body(notification.body.clone())
            {
                Ok(e) => e,
                Err(e) => {
                    return SendResult {
                        success: false,
                        channel: self.name.clone(),
                        error: Some(format!("build error: {e}")),
                    };
                }
            };

            let creds = Credentials::new(self.username.clone(), self.password.clone());
            let mailer = match AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.smtp_host)
            {
                Ok(m) => m.port(self.smtp_port).credentials(creds).build(),
                Err(e) => {
                    return SendResult {
                        success: false,
                        channel: self.name.clone(),
                        error: Some(format!("SMTP error: {e}")),
                    };
                }
            };

            if let Err(e) = mailer.send(email).await {
                return SendResult {
                    success: false,
                    channel: self.name.clone(),
                    error: Some(format!("send error: {e}")),
                };
            }
        }

        SendResult {
            success: true,
            channel: self.name.clone(),
            error: None,
        }
    }
}
