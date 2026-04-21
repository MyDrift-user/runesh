//! Email notification channel via SMTP (requires `email` feature).
//!
//! The default transport is `Tls::Required`, so TLS on the control channel
//! is mandatory. STARTTLS (`Tls::Opportunistic`) is also acceptable but must
//! be explicitly opted into. Upgrading always requires a working trust store
//! on the host (rustls uses the platform root store via `rustls-native-certs`
//! or the compiled-in webpki-roots, depending on the feature set of lettre).

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::{Notification, NotificationChannel, SendResult};

/// Maximum notification body accepted for email delivery (1 MiB).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// TLS mode for an SMTP connection.
#[derive(Debug, Clone, Copy, Default)]
pub enum SmtpTls {
    /// Implicit TLS (required). Default.
    #[default]
    Required,
    /// Opportunistic STARTTLS upgrade. Allows cleartext fallback.
    /// Opt-in: unsafe on untrusted networks.
    Opportunistic,
}

/// SMTP email channel.
pub struct EmailChannel {
    pub name: String,
    pub from: String,
    pub to: Vec<String>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    pub tls: SmtpTls,
}

impl EmailChannel {
    /// Construct a channel using required implicit TLS (recommended).
    pub fn new(
        name: impl Into<String>,
        from: impl Into<String>,
        to: Vec<String>,
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            from: from.into(),
            to,
            smtp_host: host.into(),
            smtp_port: port,
            username: username.into(),
            password: password.into(),
            tls: SmtpTls::Required,
        }
    }

    pub fn with_tls(mut self, tls: SmtpTls) -> Self {
        self.tls = tls;
        self
    }
}

/// Errors returned by email channel construction/send.
#[derive(Debug, thiserror::Error)]
pub enum NotifyEmailError {
    #[error("invalid address '{0}': {1}")]
    InvalidAddress(String, String),
    #[error("body too large: {0} > {1}")]
    BodyTooLarge(usize, usize),
    #[error("build message: {0}")]
    Build(String),
    #[error("smtp: {0}")]
    Smtp(String),
}

#[async_trait]
impl NotificationChannel for EmailChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, notification: &Notification) -> SendResult {
        let subject = format!("[{}] {}", notification.severity_label(), notification.title);

        // Cap the body.
        if notification.body.len() > MAX_BODY_BYTES {
            return SendResult {
                success: false,
                channel: self.name.clone(),
                error: Some(format!(
                    "body too large: {} > {}",
                    notification.body.len(),
                    MAX_BODY_BYTES
                )),
            };
        }

        let from_addr: lettre::Address = match self.from.parse() {
            Ok(a) => a,
            Err(e) => {
                return SendResult {
                    success: false,
                    channel: self.name.clone(),
                    error: Some(format!("invalid from address '{}': {e}", self.from)),
                };
            }
        };

        // Validate all recipients up front.
        let mut recipients: Vec<lettre::Address> = Vec::with_capacity(self.to.len());
        for r in &self.to {
            match r.parse() {
                Ok(a) => recipients.push(a),
                Err(e) => {
                    return SendResult {
                        success: false,
                        channel: self.name.clone(),
                        error: Some(format!("invalid to address '{r}': {e}")),
                    };
                }
            }
        }

        let tls_params = match TlsParameters::new(self.smtp_host.clone()) {
            Ok(p) => p,
            Err(e) => {
                return SendResult {
                    success: false,
                    channel: self.name.clone(),
                    error: Some(format!("tls params: {e}")),
                };
            }
        };
        let tls = match self.tls {
            SmtpTls::Required => Tls::Required(tls_params),
            SmtpTls::Opportunistic => Tls::Opportunistic(tls_params),
        };

        let creds = Credentials::new(self.username.clone(), self.password.clone());
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.smtp_host)
            .port(self.smtp_port)
            .tls(tls)
            .credentials(creds)
            .build();

        for recipient in recipients {
            let email = match Message::builder()
                .from(lettre::message::Mailbox::new(None, from_addr.clone()))
                .to(lettre::message::Mailbox::new(None, recipient.clone()))
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
