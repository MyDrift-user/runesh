#![deny(unsafe_code)]
//! Encrypted vault with typed entries for password management.
//!
//! Secrets are encrypted at rest with ChaCha20-Poly1305 using a
//! 256-bit master key. Each secret gets a unique nonce.
//!
//! Supports structured entry types: logins, API keys, SSH keys,
//! TOTP secrets, passkeys, certificates, WireGuard keys, database
//! credentials, credit cards, and custom key-value pairs.

pub mod entry;

pub use entry::{
    ApiKeyEntry, CardEntry, CertificateEntry, CustomEntry, DatabaseEntry, EntryContent, LoginEntry,
    PasskeyEntry, SecureNoteEntry, SshKeyEntry, TotpEntry, VaultEntry, WireguardKeyEntry,
};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A sealed (encrypted) secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    /// Secret identifier.
    pub id: String,
    /// Encrypted value (base64).
    pub ciphertext: String,
    /// Nonce used for encryption (base64).
    pub nonce: String,
    /// When this secret was stored.
    pub created_at: DateTime<Utc>,
    /// When this secret was last rotated.
    pub rotated_at: Option<DateTime<Utc>>,
    /// When this secret expires (optional).
    pub expires_at: Option<DateTime<Utc>>,
    /// Secret type label.
    #[serde(default)]
    pub secret_type: String,
}

/// An encrypted vault backed by a 256-bit master key.
pub struct Vault {
    cipher: ChaCha20Poly1305,
    secrets: std::collections::HashMap<String, SealedSecret>,
}

impl Vault {
    /// Create a new vault with a random master key.
    pub fn new() -> (Self, [u8; 32]) {
        let key = ChaCha20Poly1305::generate_key(&mut OsRng);
        let key_bytes: [u8; 32] = key.into();
        let cipher = ChaCha20Poly1305::new(&key);
        (
            Self {
                cipher,
                secrets: std::collections::HashMap::new(),
            },
            key_bytes,
        )
    }

    /// Derive a master key from a password using Argon2id and open a vault.
    /// Returns (vault, salt) where salt must be persisted for reopening.
    pub fn from_password(password: &[u8]) -> Result<(Self, [u8; 16]), VaultError> {
        let mut salt = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);
        let vault = Self::from_password_and_salt(password, &salt)?;
        Ok((vault, salt))
    }

    /// Reopen a vault with a password and a previously persisted salt.
    pub fn from_password_and_salt(password: &[u8], salt: &[u8; 16]) -> Result<Self, VaultError> {
        let mut key = [0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(password, salt, &mut key)
            .map_err(|_| VaultError::EncryptionFailed)?;
        Ok(Self::open(&key))
    }

    /// Open a vault with an existing master key.
    pub fn open(master_key: &[u8; 32]) -> Self {
        let cipher = ChaCha20Poly1305::new(master_key.into());
        Self {
            cipher,
            secrets: std::collections::HashMap::new(),
        }
    }

    /// Store a secret.
    pub fn put(
        &mut self,
        id: &str,
        plaintext: &[u8],
        secret_type: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), VaultError> {
        let nonce_bytes = chacha20poly1305::aead::OsRng.next_u64().to_le_bytes();
        let mut nonce_arr = [0u8; 12];
        nonce_arr[..8].copy_from_slice(&nonce_bytes);
        let nonce = Nonce::from(nonce_arr);

        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| VaultError::EncryptionFailed)?;

        let b64 = base64::engine::general_purpose::STANDARD;
        let now = Utc::now();
        let is_rotation = self.secrets.contains_key(id);

        self.secrets.insert(
            id.to_string(),
            SealedSecret {
                id: id.to_string(),
                ciphertext: b64.encode(&ciphertext),
                nonce: b64.encode(nonce_arr),
                created_at: if is_rotation {
                    self.secrets[id].created_at
                } else {
                    now
                },
                rotated_at: if is_rotation { Some(now) } else { None },
                expires_at,
                secret_type: secret_type.to_string(),
            },
        );

        Ok(())
    }

    /// Retrieve and decrypt a secret.
    pub fn get(&self, id: &str) -> Result<Vec<u8>, VaultError> {
        let sealed = self
            .secrets
            .get(id)
            .ok_or_else(|| VaultError::NotFound(id.to_string()))?;

        // Check expiry
        if let Some(expires) = sealed.expires_at {
            if Utc::now() > expires {
                return Err(VaultError::Expired(id.to_string()));
            }
        }

        let b64 = base64::engine::general_purpose::STANDARD;
        let ciphertext = b64
            .decode(&sealed.ciphertext)
            .map_err(|_| VaultError::CorruptData)?;
        let nonce_bytes = b64
            .decode(&sealed.nonce)
            .map_err(|_| VaultError::CorruptData)?;

        let nonce_arr: [u8; 12] = nonce_bytes
            .try_into()
            .map_err(|_| VaultError::CorruptData)?;
        let nonce = Nonce::from(nonce_arr);

        self.cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| VaultError::DecryptionFailed)
    }

    /// Delete a secret.
    pub fn delete(&mut self, id: &str) -> bool {
        self.secrets.remove(id).is_some()
    }

    /// List all secret IDs and their metadata (without decrypting).
    pub fn list(&self) -> Vec<&SealedSecret> {
        self.secrets.values().collect()
    }

    /// Get secrets that are expired or expiring within the given duration.
    pub fn expiring_within(&self, duration: chrono::Duration) -> Vec<&SealedSecret> {
        let cutoff = Utc::now() + duration;
        self.secrets
            .values()
            .filter(|s| matches!(s.expires_at, Some(exp) if exp <= cutoff))
            .collect()
    }

    /// Number of stored secrets.
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Export all sealed secrets as JSON (for persistence).
    pub fn export(&self) -> Result<String, serde_json::Error> {
        let secrets: Vec<&SealedSecret> = self.secrets.values().collect();
        serde_json::to_string_pretty(&secrets)
    }

    /// Store a typed vault entry (encrypts the full JSON representation).
    pub fn put_entry(
        &mut self,
        id: &str,
        entry: &VaultEntry,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), VaultError> {
        let bytes = entry.to_bytes().map_err(|_| VaultError::EncryptionFailed)?;
        self.put(id, &bytes, entry.entry_type(), expires_at)
    }

    /// Retrieve and decrypt a typed vault entry.
    pub fn get_entry(&self, id: &str) -> Result<VaultEntry, VaultError> {
        let bytes = self.get(id)?;
        VaultEntry::from_bytes(&bytes).map_err(|_| VaultError::CorruptData)
    }

    /// List entries filtered by type.
    pub fn list_by_type(&self, entry_type: &str) -> Vec<&SealedSecret> {
        self.secrets
            .values()
            .filter(|s| s.secret_type == entry_type)
            .collect()
    }

    /// Search entries by name (decrypts each to check, use sparingly).
    pub fn search(&self, query: &str) -> Vec<(String, VaultEntry)> {
        let query_lower = query.to_lowercase();
        self.secrets
            .keys()
            .filter_map(|id| {
                self.get_entry(id).ok().and_then(|entry| {
                    if entry.name.to_lowercase().contains(&query_lower)
                        || entry
                            .tags
                            .iter()
                            .any(|t| t.to_lowercase().contains(&query_lower))
                        || entry
                            .url
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&query_lower)
                    {
                        Some((id.clone(), entry))
                    } else {
                        None
                    }
                })
            })
            .collect()
    }

    /// Import sealed secrets from JSON.
    pub fn import(&mut self, json: &str) -> Result<usize, VaultError> {
        let secrets: Vec<SealedSecret> =
            serde_json::from_str(json).map_err(|_| VaultError::CorruptData)?;
        let count = secrets.len();
        for s in secrets {
            self.secrets.insert(s.id.clone(), s);
        }
        Ok(count)
    }
}

use rand::RngCore;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("secret not found: {0}")]
    NotFound(String),
    #[error("secret expired: {0}")]
    Expired(String),
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed (wrong key or corrupt data)")]
    DecryptionFailed,
    #[error("corrupt data")]
    CorruptData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get() {
        let (mut vault, _key) = Vault::new();
        vault
            .put("db-password", b"supersecret", "password", None)
            .unwrap();

        let plaintext = vault.get("db-password").unwrap();
        assert_eq!(plaintext, b"supersecret");
    }

    #[test]
    fn wrong_key_fails() {
        let (mut vault, _key) = Vault::new();
        vault.put("secret", b"data", "generic", None).unwrap();

        // Export and try to decrypt with a different key
        let sealed = vault.secrets.get("secret").unwrap().clone();
        let wrong_key = [0u8; 32];
        let mut wrong_vault = Vault::open(&wrong_key);
        wrong_vault.secrets.insert("secret".into(), sealed);

        assert!(wrong_vault.get("secret").is_err());
    }

    #[test]
    fn not_found() {
        let (vault, _) = Vault::new();
        assert!(matches!(vault.get("nope"), Err(VaultError::NotFound(_))));
    }

    #[test]
    fn delete_secret() {
        let (mut vault, _) = Vault::new();
        vault.put("tmp", b"data", "generic", None).unwrap();
        assert!(vault.delete("tmp"));
        assert!(!vault.delete("tmp"));
        assert!(vault.get("tmp").is_err());
    }

    #[test]
    fn rotation_preserves_created_at() {
        let (mut vault, _) = Vault::new();
        vault.put("key", b"v1", "api_key", None).unwrap();
        let created = vault.secrets["key"].created_at;

        std::thread::sleep(std::time::Duration::from_millis(10));
        vault.put("key", b"v2", "api_key", None).unwrap();

        assert_eq!(vault.secrets["key"].created_at, created);
        assert!(vault.secrets["key"].rotated_at.is_some());
        assert_eq!(vault.get("key").unwrap(), b"v2");
    }

    #[test]
    fn expired_secret_rejected() {
        let (mut vault, _) = Vault::new();
        let past = Utc::now() - chrono::Duration::hours(1);
        vault.put("old", b"data", "generic", Some(past)).unwrap();
        assert!(matches!(vault.get("old"), Err(VaultError::Expired(_))));
    }

    #[test]
    fn list_secrets() {
        let (mut vault, _) = Vault::new();
        vault.put("a", b"1", "password", None).unwrap();
        vault.put("b", b"2", "api_key", None).unwrap();
        assert_eq!(vault.list().len(), 2);
    }

    #[test]
    fn export_import() {
        let (mut vault, key) = Vault::new();
        vault.put("secret", b"value", "generic", None).unwrap();

        let exported = vault.export().unwrap();
        let mut vault2 = Vault::open(&key);
        vault2.import(&exported).unwrap();

        assert_eq!(vault2.get("secret").unwrap(), b"value");
    }
}
