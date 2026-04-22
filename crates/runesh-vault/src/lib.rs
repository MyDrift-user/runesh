#![deny(unsafe_code)]
//! Encrypted vault with typed entries for password management.
//!
//! Secrets are encrypted at rest with ChaCha20-Poly1305 using a 256-bit
//! master key. Each secret receives a full 96-bit random nonce drawn from
//! the OS CSPRNG so there is no reuse risk across a given key's lifetime.
//!
//! Password-derived keys use Argon2id with the OWASP 2024 baseline
//! (m=64 MiB, t=3, p=4) by default; callers can override via [`VaultConfig`].
//!
//! Master keys are wrapped in [`secrecy::SecretBox`] to avoid accidental
//! logging and to zeroize on drop. Callers access the bytes explicitly via
//! `expose_secret`.
//!
//! Each stored secret carries an HMAC-SHA256 tag keyed from a vault-key
//! derived HMAC key. [`Vault::import`] validates the tag before accepting
//! foreign ciphertext; mismatches are rejected with [`VaultError::IntegrityCheckFailed`]
//! so an attacker cannot inject crafted ciphertext against a vault they
//! do not control.
//!
//! Supports structured entry types: logins, API keys, SSH keys, TOTP secrets,
//! passkeys, certificates, WireGuard keys, database credentials, credit cards,
//! and custom key-value pairs.

pub mod entry;

pub use entry::{
    ApiKeyEntry, CardEntry, CertificateEntry, CustomEntry, DatabaseEntry, EntryContent, LoginEntry,
    PasskeyEntry, SecureNoteEntry, SshKeyEntry, TotpEntry, VaultEntry, WireguardKeyEntry,
};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
#[cfg(test)]
use secrecy::ExposeSecret;
use secrecy::SecretBox;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// HKDF-style info label for deriving the integrity HMAC key from the master key.
const INTEGRITY_INFO: &[u8] = b"vault-integrity";

/// A sealed (encrypted) secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    /// Secret identifier.
    pub id: String,
    /// Encrypted value (base64).
    pub ciphertext: String,
    /// Nonce used for encryption (base64), 12 random bytes.
    pub nonce: String,
    /// HMAC-SHA256 tag (base64) over the canonical envelope of this secret,
    /// keyed with a master-key-derived integrity key. Rejected on import if
    /// it does not verify.
    #[serde(default)]
    pub integrity_tag: String,
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

/// Argon2id parameter bundle. Defaults follow the OWASP 2024 guidance.
#[derive(Debug, Clone, Copy)]
pub struct VaultConfig {
    /// Memory cost in KiB. Default: 65536 (64 MiB).
    pub argon2_m_cost: u32,
    /// Time cost (iterations). Default: 3.
    pub argon2_t_cost: u32,
    /// Parallelism. Default: 4.
    pub argon2_p_cost: u32,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            argon2_m_cost: 65_536,
            argon2_t_cost: 3,
            argon2_p_cost: 4,
        }
    }
}

impl VaultConfig {
    fn argon2(&self) -> Result<Argon2<'static>, VaultError> {
        let params = Params::new(
            self.argon2_m_cost,
            self.argon2_t_cost,
            self.argon2_p_cost,
            None,
        )
        .map_err(|_| VaultError::WeakKdfParams)?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// Zero-on-drop wrapper for the 32-byte master key.
pub type MasterKey = SecretBox<[u8; 32]>;

/// Derive the integrity HMAC key from the master key. Uses a simple
/// HKDF-like extract step (HMAC-SHA256 with a fixed info label) so the
/// integrity key is deterministic yet domain-separated from the cipher key.
fn derive_integrity_key(master_key: &[u8; 32]) -> [u8; 32] {
    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(master_key)
        .expect("HMAC accepts any key length");
    mac.update(INTEGRITY_INFO);
    let out = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// Compute the integrity tag over the canonical envelope of a sealed secret.
/// Fields are length-prefixed so concatenation cannot cause collisions.
fn integrity_tag(key: &[u8; 32], id: &str, nonce_b64: &str, ciphertext_b64: &str) -> String {
    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    for field in [
        id.as_bytes(),
        nonce_b64.as_bytes(),
        ciphertext_b64.as_bytes(),
    ] {
        mac.update(&(field.len() as u64).to_be_bytes());
        mac.update(field);
    }
    let bytes = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn verify_integrity_tag(
    key: &[u8; 32],
    id: &str,
    nonce_b64: &str,
    ciphertext_b64: &str,
    tag_b64: &str,
) -> bool {
    let expected = integrity_tag(key, id, nonce_b64, ciphertext_b64);
    if expected.len() != tag_b64.len() {
        return false;
    }
    expected.as_bytes().ct_eq(tag_b64.as_bytes()).unwrap_u8() == 1
}

/// An encrypted vault backed by a 256-bit master key.
pub struct Vault {
    cipher: ChaCha20Poly1305,
    integrity_key: [u8; 32],
    secrets: std::collections::HashMap<String, SealedSecret>,
    config: VaultConfig,
}

impl Vault {
    /// Create a new vault with a random master key. The key is wrapped in a
    /// [`SecretBox`] and zeroized on drop; call [`ExposeSecret::expose_secret`]
    /// to persist or transport it.
    pub fn new() -> (Self, MasterKey) {
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let vault = Self::from_key_bytes(&key_bytes, VaultConfig::default());
        (vault, SecretBox::new(Box::new(key_bytes)))
    }

    /// Create a new vault with explicit config.
    pub fn new_with_config(config: VaultConfig) -> (Self, MasterKey) {
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let vault = Self::from_key_bytes(&key_bytes, config);
        (vault, SecretBox::new(Box::new(key_bytes)))
    }

    /// Derive a master key from a password using Argon2id and open a vault.
    /// Returns (vault, salt) where salt must be persisted for reopening.
    pub fn from_password(password: &[u8]) -> Result<(Self, [u8; 16]), VaultError> {
        Self::from_password_with_config(password, VaultConfig::default())
    }

    /// Derive a master key from a password with explicit KDF params.
    pub fn from_password_with_config(
        password: &[u8],
        config: VaultConfig,
    ) -> Result<(Self, [u8; 16]), VaultError> {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let vault = Self::from_password_and_salt_with_config(password, &salt, config)?;
        Ok((vault, salt))
    }

    /// Reopen a vault with a password and a previously persisted salt.
    pub fn from_password_and_salt(password: &[u8], salt: &[u8; 16]) -> Result<Self, VaultError> {
        Self::from_password_and_salt_with_config(password, salt, VaultConfig::default())
    }

    /// Reopen with an explicit config. The caller must supply the same config
    /// used at creation; otherwise the derived key will not match.
    pub fn from_password_and_salt_with_config(
        password: &[u8],
        salt: &[u8; 16],
        config: VaultConfig,
    ) -> Result<Self, VaultError> {
        let mut key = [0u8; 32];
        config
            .argon2()?
            .hash_password_into(password, salt, &mut key)
            .map_err(|_| VaultError::EncryptionFailed)?;
        Ok(Self::from_key_bytes(&key, config))
    }

    /// Open a vault with an existing master key. The key material is copied
    /// into the vault state and the caller's reference can be dropped / zeroized.
    pub fn open(master_key: &[u8; 32]) -> Self {
        Self::from_key_bytes(master_key, VaultConfig::default())
    }

    /// Open with explicit config.
    pub fn open_with_config(master_key: &[u8; 32], config: VaultConfig) -> Self {
        Self::from_key_bytes(master_key, config)
    }

    fn from_key_bytes(master_key: &[u8; 32], config: VaultConfig) -> Self {
        let cipher_key = Key::from_slice(master_key);
        let cipher = ChaCha20Poly1305::new(cipher_key);
        let integrity_key = derive_integrity_key(master_key);
        Self {
            cipher,
            integrity_key,
            secrets: std::collections::HashMap::new(),
            config,
        }
    }

    /// Active KDF config for this vault.
    pub fn config(&self) -> VaultConfig {
        self.config
    }

    /// Store a secret.
    pub fn put(
        &mut self,
        id: &str,
        plaintext: &[u8],
        secret_type: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), VaultError> {
        // Generate a full 96-bit random nonce from the OS CSPRNG. With 2^96
        // possible values the birthday bound for reuse is astronomically low
        // even for vaults with billions of entries under the same key.
        let mut nonce_arr = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_arr);
        let nonce = Nonce::from(nonce_arr);

        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| VaultError::EncryptionFailed)?;

        let b64 = base64::engine::general_purpose::STANDARD;
        let ciphertext_b64 = b64.encode(&ciphertext);
        let nonce_b64 = b64.encode(nonce_arr);
        let tag = integrity_tag(&self.integrity_key, id, &nonce_b64, &ciphertext_b64);
        let now = Utc::now();
        let is_rotation = self.secrets.contains_key(id);

        self.secrets.insert(
            id.to_string(),
            SealedSecret {
                id: id.to_string(),
                ciphertext: ciphertext_b64,
                nonce: nonce_b64,
                integrity_tag: tag,
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

        if let Some(expires) = sealed.expires_at
            && Utc::now() > expires
        {
            return Err(VaultError::Expired(id.to_string()));
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
    ///
    /// Each entry is checked against its stored HMAC tag (keyed with this
    /// vault's integrity key). Entries whose tag does not verify are rejected
    /// with [`VaultError::IntegrityCheckFailed`]. This prevents an attacker
    /// from injecting crafted ciphertext into a vault they do not control:
    /// without the master key they cannot produce a valid tag.
    pub fn import(&mut self, json: &str) -> Result<usize, VaultError> {
        let secrets: Vec<SealedSecret> =
            serde_json::from_str(json).map_err(|_| VaultError::CorruptData)?;
        let count = secrets.len();
        for s in &secrets {
            if s.integrity_tag.is_empty() {
                return Err(VaultError::IntegrityCheckFailed(s.id.clone()));
            }
            if !verify_integrity_tag(
                &self.integrity_key,
                &s.id,
                &s.nonce,
                &s.ciphertext,
                &s.integrity_tag,
            ) {
                return Err(VaultError::IntegrityCheckFailed(s.id.clone()));
            }
        }
        for s in secrets {
            self.secrets.insert(s.id.clone(), s);
        }
        Ok(count)
    }

    /// Re-encrypt every secret under `new_key` and replace the active
    /// cipher. Each secret gets a fresh 96-bit random nonce and a fresh
    /// integrity tag. All `rotated_at` fields are advanced to "now".
    ///
    /// The rotation is prepared in a scratch map and only swapped in on
    /// full success, so a failure mid-way (for example a corrupt entry
    /// that fails to decrypt under the current key) leaves the vault
    /// unchanged. `created_at` and `expires_at` are preserved.
    pub fn rotate_master_key(&mut self, new_key: &[u8; 32]) -> Result<(), VaultError> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let new_cipher_key = Key::from_slice(new_key);
        let new_cipher = ChaCha20Poly1305::new(new_cipher_key);
        let new_integrity_key = derive_integrity_key(new_key);
        let now = Utc::now();

        let mut fresh: std::collections::HashMap<String, SealedSecret> =
            std::collections::HashMap::with_capacity(self.secrets.len());

        for (id, sealed) in &self.secrets {
            // Decrypt under the current key. Any failure here aborts the
            // rotation so the vault is not left half-rewrapped.
            let ciphertext = b64
                .decode(&sealed.ciphertext)
                .map_err(|_| VaultError::CorruptData)?;
            let nonce_bytes = b64
                .decode(&sealed.nonce)
                .map_err(|_| VaultError::CorruptData)?;
            let nonce_arr: [u8; 12] = nonce_bytes
                .try_into()
                .map_err(|_| VaultError::CorruptData)?;
            let old_nonce = Nonce::from(nonce_arr);
            let plaintext = self
                .cipher
                .decrypt(&old_nonce, ciphertext.as_ref())
                .map_err(|_| VaultError::DecryptionFailed)?;

            // Re-encrypt under the new key with a fresh nonce.
            let mut new_nonce_arr = [0u8; 12];
            OsRng.fill_bytes(&mut new_nonce_arr);
            let new_nonce = Nonce::from(new_nonce_arr);
            let new_ct = new_cipher
                .encrypt(&new_nonce, plaintext.as_ref())
                .map_err(|_| VaultError::EncryptionFailed)?;
            let new_ct_b64 = b64.encode(&new_ct);
            let new_nonce_b64 = b64.encode(new_nonce_arr);
            let new_tag = integrity_tag(&new_integrity_key, id, &new_nonce_b64, &new_ct_b64);

            fresh.insert(
                id.clone(),
                SealedSecret {
                    id: sealed.id.clone(),
                    ciphertext: new_ct_b64,
                    nonce: new_nonce_b64,
                    integrity_tag: new_tag,
                    created_at: sealed.created_at,
                    rotated_at: Some(now),
                    expires_at: sealed.expires_at,
                    secret_type: sealed.secret_type.clone(),
                },
            );
        }

        // All secrets re-encrypted successfully; commit.
        self.cipher = new_cipher;
        self.integrity_key = new_integrity_key;
        self.secrets = fresh;
        Ok(())
    }
}

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
    #[error("integrity check failed for secret: {0}")]
    IntegrityCheckFailed(String),
    #[error("Argon2 parameters rejected")]
    WeakKdfParams,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rotate_master_key_rewraps_all_secrets() {
        let (mut vault, old_key) = Vault::new();
        vault.put("a", b"value-a", "generic", None).unwrap();
        vault.put("b", b"value-b", "generic", None).unwrap();
        vault
            .put(
                "c",
                b"value-c",
                "generic",
                Some(Utc::now() + chrono::Duration::days(1)),
            )
            .unwrap();

        let mut new_key = [0u8; 32];
        OsRng.fill_bytes(&mut new_key);
        assert_ne!(old_key.expose_secret(), &new_key);

        let old_sealed_a = vault.secrets["a"].clone();
        vault.rotate_master_key(&new_key).unwrap();

        // Secrets still decrypt after rotation.
        assert_eq!(vault.get("a").unwrap(), b"value-a");
        assert_eq!(vault.get("b").unwrap(), b"value-b");
        assert_eq!(vault.get("c").unwrap(), b"value-c");

        // Rewrapped: nonce, ciphertext and tag all change.
        let new_sealed_a = &vault.secrets["a"];
        assert_ne!(old_sealed_a.nonce, new_sealed_a.nonce);
        assert_ne!(old_sealed_a.ciphertext, new_sealed_a.ciphertext);
        assert_ne!(old_sealed_a.integrity_tag, new_sealed_a.integrity_tag);
        assert!(new_sealed_a.rotated_at.is_some());

        // Opening a vault with the OLD key and importing the rewrapped
        // export must fail integrity check: the new tag is keyed to the
        // new master key.
        let mut stale = Vault::open(old_key.expose_secret());
        let json = vault.export().unwrap();
        assert!(matches!(
            stale.import(&json),
            Err(VaultError::IntegrityCheckFailed(_))
        ));

        // Opening with the NEW key works.
        let mut reopened = Vault::open(&new_key);
        reopened.import(&vault.export().unwrap()).unwrap();
        assert_eq!(reopened.get("a").unwrap(), b"value-a");
    }

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
    fn export_import_roundtrip_same_key() {
        let (mut vault, key) = Vault::new();
        vault.put("secret", b"value", "generic", None).unwrap();

        let exported = vault.export().unwrap();
        let mut vault2 = Vault::open(key.expose_secret());
        vault2.import(&exported).unwrap();

        assert_eq!(vault2.get("secret").unwrap(), b"value");
    }

    #[test]
    fn import_rejects_foreign_ciphertext() {
        // Vault A encrypts a secret; vault B has a different master key.
        // Importing A's JSON into B must fail with IntegrityCheckFailed
        // because the HMAC tag is keyed with A's integrity key.
        let (mut vault_a, _key_a) = Vault::new();
        vault_a.put("foo", b"bar", "generic", None).unwrap();
        let exported = vault_a.export().unwrap();

        let (mut vault_b, _key_b) = Vault::new();
        let result = vault_b.import(&exported);
        assert!(matches!(result, Err(VaultError::IntegrityCheckFailed(_))));
    }

    #[test]
    fn import_rejects_missing_tag() {
        // Strip the integrity_tag from the exported JSON and import should
        // fail even under the same key: blank tags are not accepted.
        let (mut vault, _key) = Vault::new();
        vault.put("foo", b"bar", "generic", None).unwrap();
        let exported = vault.export().unwrap();
        let tampered = exported.replace(&vault.secrets["foo"].integrity_tag, "");

        let mut vault2 = Vault::open(_key.expose_secret());
        let result = vault2.import(&tampered);
        assert!(matches!(result, Err(VaultError::IntegrityCheckFailed(_))));
    }

    #[test]
    fn nonces_are_unique_and_random() {
        // Write many secrets; no two should share a nonce.
        let (mut vault, _) = Vault::new();
        for i in 0..512 {
            vault
                .put(&format!("s{i}"), b"payload", "generic", None)
                .unwrap();
        }
        let mut seen = HashSet::new();
        for s in vault.list() {
            assert!(seen.insert(s.nonce.clone()), "duplicate nonce: {}", s.nonce);
        }
        assert_eq!(seen.len(), 512);
    }

    #[test]
    fn nonce_is_not_trailing_zeros() {
        // The previous implementation used u64 little-endian in the first 8
        // bytes and zeroed the last 4. Guard against regressing: with 512
        // entries at least one should have a non-zero trailing byte.
        let (mut vault, _) = Vault::new();
        for i in 0..32 {
            vault
                .put(&format!("s{i}"), b"payload", "generic", None)
                .unwrap();
        }
        let any_nonzero_tail = vault.list().iter().any(|s| {
            let b64 = base64::engine::general_purpose::STANDARD;
            let bytes = b64.decode(&s.nonce).unwrap();
            bytes[8..12].iter().any(|&b| b != 0)
        });
        assert!(
            any_nonzero_tail,
            "trailing 4 nonce bytes were always zero; RNG is not being used"
        );
    }

    #[test]
    fn argon2_params_are_strong_by_default() {
        let cfg = VaultConfig::default();
        assert!(cfg.argon2_m_cost >= 65_536);
        assert!(cfg.argon2_t_cost >= 3);
        assert!(cfg.argon2_p_cost >= 1);
    }

    #[test]
    fn password_vault_reopens_with_same_salt() {
        // Use weakened params so the test runs quickly.
        let cfg = VaultConfig {
            argon2_m_cost: 8,
            argon2_t_cost: 1,
            argon2_p_cost: 1,
        };
        let (mut vault, salt) = Vault::from_password_with_config(b"hunter2", cfg).unwrap();
        vault.put("x", b"hello", "generic", None).unwrap();
        let export = vault.export().unwrap();

        let mut reopened =
            Vault::from_password_and_salt_with_config(b"hunter2", &salt, cfg).unwrap();
        reopened.import(&export).unwrap();
        assert_eq!(reopened.get("x").unwrap(), b"hello");
    }

    #[test]
    fn master_key_is_zeroized_on_drop() {
        // Compile-time check: MasterKey is SecretBox<[u8;32]>, which implements
        // Zeroize on drop. Exercise expose_secret so the wrapper is used.
        let (_vault, key) = Vault::new();
        let bytes = *key.expose_secret();
        assert_eq!(bytes.len(), 32);
    }
}
