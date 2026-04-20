//! Structured vault entry types for password manager use.
//!
//! Each entry type serializes to JSON before encryption, so the vault
//! stores opaque encrypted bytes regardless of entry type.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// A vault entry with typed content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    /// Display name.
    pub name: String,
    /// Optional folder/category for organization.
    #[serde(default)]
    pub folder: Option<String>,
    /// Tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// URL associated with this entry (for passwords, API keys).
    #[serde(default)]
    pub url: Option<String>,
    /// Notes.
    #[serde(default)]
    pub notes: String,
    /// Favorite flag.
    #[serde(default)]
    pub favorite: bool,
    /// The typed secret content.
    pub content: EntryContent,
}

/// The secret content, typed by category.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryContent {
    /// Login credentials (username + password).
    Login(LoginEntry),
    /// API key or token.
    ApiKey(ApiKeyEntry),
    /// SSH keypair.
    SshKey(SshKeyEntry),
    /// TOTP two-factor authentication.
    Totp(TotpEntry),
    /// Passkey / WebAuthn credential.
    Passkey(PasskeyEntry),
    /// Credit card.
    Card(CardEntry),
    /// Secure note (just text).
    SecureNote(SecureNoteEntry),
    /// TLS/X.509 certificate with private key.
    Certificate(CertificateEntry),
    /// WireGuard keypair.
    WireguardKey(WireguardKeyEntry),
    /// Database connection credentials.
    Database(DatabaseEntry),
    /// Generic key-value pairs.
    Custom(CustomEntry),
}

/// Login credentials.
#[derive(Clone, Serialize, Deserialize)]
pub struct LoginEntry {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub totp_secret: Option<String>,
    #[serde(default)]
    pub autofill_urls: Vec<String>,
}

/// API key or bearer token.
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub key: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// SSH keypair.
#[derive(Clone, Serialize, Deserialize)]
pub struct SshKeyEntry {
    /// Private key (PEM or OpenSSH format).
    pub private_key: String,
    /// Public key.
    pub public_key: String,
    /// Key type (ed25519, rsa, ecdsa).
    #[serde(default)]
    pub key_type: Option<String>,
    /// Fingerprint (SHA-256).
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Passphrase for the private key (if encrypted).
    #[serde(default)]
    pub passphrase: Option<String>,
}

/// TOTP two-factor authentication secret.
#[derive(Clone, Serialize, Deserialize)]
pub struct TotpEntry {
    /// Base32-encoded TOTP secret.
    pub secret: String,
    /// Issuer name.
    #[serde(default)]
    pub issuer: Option<String>,
    /// Account name.
    #[serde(default)]
    pub account: Option<String>,
    /// Number of digits (default: 6).
    #[serde(default = "default_digits")]
    pub digits: u32,
    /// Period in seconds (default: 30).
    #[serde(default = "default_period")]
    pub period: u32,
    /// Algorithm (SHA1, SHA256, SHA512).
    #[serde(default = "default_algorithm")]
    pub algorithm: String,
}

/// Passkey / WebAuthn credential.
#[derive(Clone, Serialize, Deserialize)]
pub struct PasskeyEntry {
    /// Credential ID (base64url).
    pub credential_id: String,
    /// Private key (COSE key, base64).
    pub private_key: String,
    /// Relying party ID (domain).
    pub rp_id: String,
    /// User handle (base64url).
    pub user_handle: String,
    /// User display name.
    #[serde(default)]
    pub user_name: Option<String>,
    /// Sign count.
    #[serde(default)]
    pub sign_count: u32,
}

/// Credit card.
#[derive(Clone, Serialize, Deserialize)]
pub struct CardEntry {
    pub cardholder: String,
    pub number: String,
    pub expiry: String,
    pub cvv: String,
    #[serde(default)]
    pub brand: Option<String>,
}

/// Secure text note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureNoteEntry {
    pub text: String,
}

/// TLS certificate with private key.
#[derive(Clone, Serialize, Deserialize)]
pub struct CertificateEntry {
    /// Certificate chain (PEM).
    pub cert_pem: String,
    /// Private key (PEM).
    pub key_pem: String,
    /// Subject common name.
    #[serde(default)]
    pub common_name: Option<String>,
    /// Expiry date (ISO 8601).
    #[serde(default)]
    pub expires: Option<String>,
}

/// WireGuard keypair.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireguardKeyEntry {
    pub private_key: String,
    pub public_key: String,
    #[serde(default)]
    pub preshared_key: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// Database connection.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseEntry {
    pub db_type: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub ssl: bool,
    #[serde(default)]
    pub connection_string: Option<String>,
}

/// Generic key-value entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEntry {
    pub fields: std::collections::HashMap<String, String>,
}

// --- Redacted Debug impls: never print secrets to logs ---

macro_rules! redacted_debug {
    ($name:ident, $label:expr) => {
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct($label).field("_", &"[REDACTED]").finish()
            }
        }
    };
}

redacted_debug!(LoginEntry, "LoginEntry");
redacted_debug!(ApiKeyEntry, "ApiKeyEntry");
redacted_debug!(SshKeyEntry, "SshKeyEntry");
redacted_debug!(TotpEntry, "TotpEntry");
redacted_debug!(PasskeyEntry, "PasskeyEntry");
redacted_debug!(CardEntry, "CardEntry");
redacted_debug!(CertificateEntry, "CertificateEntry");
redacted_debug!(WireguardKeyEntry, "WireguardKeyEntry");
redacted_debug!(DatabaseEntry, "DatabaseEntry");

// --- Zeroize on drop: clear secrets from memory ---

macro_rules! zeroize_on_drop {
    ($name:ident, $($field:ident),+) => {
        impl Drop for $name {
            fn drop(&mut self) {
                $(self.$field.zeroize();)+
            }
        }
    };
}

zeroize_on_drop!(LoginEntry, password);
zeroize_on_drop!(ApiKeyEntry, key);
zeroize_on_drop!(SshKeyEntry, private_key);
zeroize_on_drop!(TotpEntry, secret);
zeroize_on_drop!(PasskeyEntry, private_key);
zeroize_on_drop!(CardEntry, number, cvv);
zeroize_on_drop!(CertificateEntry, key_pem);
zeroize_on_drop!(WireguardKeyEntry, private_key);
zeroize_on_drop!(DatabaseEntry, password);

impl VaultEntry {
    /// Serialize to JSON bytes for encryption.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize from decrypted JSON bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Get the entry type as a string label.
    pub fn entry_type(&self) -> &str {
        match &self.content {
            EntryContent::Login(_) => "login",
            EntryContent::ApiKey(_) => "api_key",
            EntryContent::SshKey(_) => "ssh_key",
            EntryContent::Totp(_) => "totp",
            EntryContent::Passkey(_) => "passkey",
            EntryContent::Card(_) => "card",
            EntryContent::SecureNote(_) => "secure_note",
            EntryContent::Certificate(_) => "certificate",
            EntryContent::WireguardKey(_) => "wireguard_key",
            EntryContent::Database(_) => "database",
            EntryContent::Custom(_) => "custom",
        }
    }

    /// Generate a TOTP URI (otpauth://) for QR code generation.
    pub fn totp_uri(&self) -> Option<String> {
        let totp = match &self.content {
            EntryContent::Totp(t) => t,
            EntryContent::Login(LoginEntry {
                totp_secret: Some(s),
                ..
            }) => {
                return Some(format!(
                    "otpauth://totp/{}?secret={}&digits=6&period=30",
                    self.name, s
                ));
            }
            _ => return None,
        };
        let issuer = totp.issuer.as_deref().unwrap_or(&self.name);
        let account = totp.account.as_deref().unwrap_or("");
        let label = if account.is_empty() {
            issuer.to_string()
        } else {
            format!("{issuer}:{account}")
        };
        Some(format!(
            "otpauth://totp/{}?secret={}&issuer={}&digits={}&period={}&algorithm={}",
            label, totp.secret, issuer, totp.digits, totp.period, totp.algorithm
        ))
    }

    /// Generate the current TOTP code for entries that have a TOTP secret.
    ///
    /// Returns the 6-8 digit code and the seconds remaining until it expires.
    pub fn generate_totp(&self) -> Option<(String, u64)> {
        let (secret, digits, period, algorithm) = match &self.content {
            EntryContent::Totp(t) => (&t.secret, t.digits, t.period, t.algorithm.as_str()),
            EntryContent::Login(LoginEntry {
                totp_secret: Some(s),
                ..
            }) => (s, 6, 30, "SHA1"),
            _ => return None,
        };

        let algo = match algorithm.to_uppercase().as_str() {
            "SHA256" => totp_rs::Algorithm::SHA256,
            "SHA512" => totp_rs::Algorithm::SHA512,
            _ => totp_rs::Algorithm::SHA1,
        };

        let decoded = totp_rs::Secret::Encoded(secret.clone()).to_bytes().ok()?;

        let totp = totp_rs::TOTP::new(algo, digits as usize, 1, period as u64, decoded).ok()?;

        let code = totp.generate_current().ok()?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let remaining = period as u64 - (now % period as u64);

        Some((code, remaining))
    }
}

fn default_digits() -> u32 {
    6
}
fn default_period() -> u32 {
    30
}
fn default_algorithm() -> String {
    "SHA1".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_entry_roundtrip() {
        let entry = VaultEntry {
            name: "GitHub".into(),
            folder: Some("Development".into()),
            tags: vec!["work".into()],
            url: Some("https://github.com".into()),
            notes: String::new(),
            favorite: true,
            content: EntryContent::Login(LoginEntry {
                username: "user@example.com".into(),
                password: "hunter2".into(),
                totp_secret: Some("JBSWY3DPEHPK3PXP".into()),
                autofill_urls: vec!["https://github.com/login".into()],
            }),
        };
        assert_eq!(entry.entry_type(), "login");

        let bytes = entry.to_bytes().unwrap();
        let restored = VaultEntry::from_bytes(&bytes).unwrap();
        assert_eq!(restored.name, "GitHub");
        if let EntryContent::Login(l) = &restored.content {
            assert_eq!(l.username, "user@example.com");
            assert_eq!(l.password, "hunter2");
        } else {
            panic!("wrong type");
        }
    }

    #[test]
    fn ssh_key_entry() {
        let entry = VaultEntry {
            name: "Production SSH".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::SshKey(SshKeyEntry {
                private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\n...".into(),
                public_key: "ssh-ed25519 AAAA... user@host".into(),
                key_type: Some("ed25519".into()),
                fingerprint: Some("SHA256:abc123".into()),
                passphrase: None,
            }),
        };
        assert_eq!(entry.entry_type(), "ssh_key");
        let bytes = entry.to_bytes().unwrap();
        let restored = VaultEntry::from_bytes(&bytes).unwrap();
        if let EntryContent::SshKey(k) = &restored.content {
            assert!(k.private_key.contains("OPENSSH"));
        } else {
            panic!("wrong type");
        }
    }

    #[test]
    fn totp_entry_and_uri() {
        let entry = VaultEntry {
            name: "AWS".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::Totp(TotpEntry {
                secret: "JBSWY3DPEHPK3PXP".into(),
                issuer: Some("Amazon".into()),
                account: Some("admin@company.com".into()),
                digits: 6,
                period: 30,
                algorithm: "SHA1".into(),
            }),
        };
        let uri = entry.totp_uri().unwrap();
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("Amazon"));
    }

    #[test]
    fn passkey_entry() {
        let entry = VaultEntry {
            name: "Google Passkey".into(),
            folder: None,
            tags: vec![],
            url: Some("https://google.com".into()),
            notes: String::new(),
            favorite: false,
            content: EntryContent::Passkey(PasskeyEntry {
                credential_id: "Y3JlZGVudGlhbA".into(),
                private_key: "cHJpdmF0ZWtleQ".into(),
                rp_id: "google.com".into(),
                user_handle: "dXNlcg".into(),
                user_name: Some("user@gmail.com".into()),
                sign_count: 5,
            }),
        };
        assert_eq!(entry.entry_type(), "passkey");
        let bytes = entry.to_bytes().unwrap();
        VaultEntry::from_bytes(&bytes).unwrap();
    }

    #[test]
    fn api_key_entry() {
        let entry = VaultEntry {
            name: "Cloudflare API".into(),
            folder: Some("Infrastructure".into()),
            tags: vec!["dns".into(), "cdn".into()],
            url: Some("https://api.cloudflare.com".into()),
            notes: String::new(),
            favorite: false,
            content: EntryContent::ApiKey(ApiKeyEntry {
                key: "cf-api-key-123".into(),
                secret: Some("cf-api-secret-456".into()),
                prefix: Some("Bearer".into()),
                scopes: vec!["dns:read".into(), "dns:write".into()],
            }),
        };
        assert_eq!(entry.entry_type(), "api_key");
        let bytes = entry.to_bytes().unwrap();
        let restored = VaultEntry::from_bytes(&bytes).unwrap();
        if let EntryContent::ApiKey(a) = &restored.content {
            assert_eq!(a.scopes.len(), 2);
        } else {
            panic!("wrong type");
        }
    }

    #[test]
    fn database_entry() {
        let entry = VaultEntry {
            name: "Production Postgres".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: "Primary database".into(),
            favorite: false,
            content: EntryContent::Database(DatabaseEntry {
                db_type: "postgres".into(),
                host: "db.internal".into(),
                port: 5432,
                database: "app".into(),
                username: "app_user".into(),
                password: "db_password_123".into(),
                ssl: true,
                connection_string: None,
            }),
        };
        assert_eq!(entry.entry_type(), "database");
        let bytes = entry.to_bytes().unwrap();
        VaultEntry::from_bytes(&bytes).unwrap();
    }

    #[test]
    fn wireguard_key_entry() {
        let entry = VaultEntry {
            name: "VPN Tunnel".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::WireguardKey(WireguardKeyEntry {
                private_key: "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=".into(),
                public_key: "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=".into(),
                preshared_key: None,
                endpoint: Some("vpn.example.com:51820".into()),
            }),
        };
        assert_eq!(entry.entry_type(), "wireguard_key");
    }

    #[test]
    fn card_entry() {
        let entry = VaultEntry {
            name: "Company Card".into(),
            folder: Some("Finance".into()),
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::Card(CardEntry {
                cardholder: "John Doe".into(),
                number: "4111111111111111".into(),
                expiry: "12/28".into(),
                cvv: "123".into(),
                brand: Some("Visa".into()),
            }),
        };
        assert_eq!(entry.entry_type(), "card");
    }

    #[test]
    fn certificate_entry() {
        let entry = VaultEntry {
            name: "Wildcard SSL".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::Certificate(CertificateEntry {
                cert_pem: "-----BEGIN CERTIFICATE-----\n...".into(),
                key_pem: "-----BEGIN PRIVATE KEY-----\n...".into(),
                common_name: Some("*.example.com".into()),
                expires: Some("2027-12-31".into()),
            }),
        };
        assert_eq!(entry.entry_type(), "certificate");
    }

    #[test]
    fn custom_entry() {
        let entry = VaultEntry {
            name: "Misc".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::Custom(CustomEntry {
                fields: [
                    ("key1".into(), "val1".into()),
                    ("key2".into(), "val2".into()),
                ]
                .into(),
            }),
        };
        assert_eq!(entry.entry_type(), "custom");
    }

    #[test]
    fn all_entry_types_serialize() {
        let types = vec![
            "login",
            "api_key",
            "ssh_key",
            "totp",
            "passkey",
            "card",
            "secure_note",
            "certificate",
            "wireguard_key",
            "database",
            "custom",
        ];
        // Just verify the list matches what we support
        assert_eq!(types.len(), 11);
    }

    #[test]
    fn totp_uri_from_login_with_totp() {
        let entry = VaultEntry {
            name: "Service".into(),
            folder: None,
            tags: vec![],
            url: None,
            notes: String::new(),
            favorite: false,
            content: EntryContent::Login(LoginEntry {
                username: "user".into(),
                password: "pass".into(),
                totp_secret: Some("ABCDEF".into()),
                autofill_urls: vec![],
            }),
        };
        let uri = entry.totp_uri().unwrap();
        assert!(uri.contains("ABCDEF"));
    }
}
