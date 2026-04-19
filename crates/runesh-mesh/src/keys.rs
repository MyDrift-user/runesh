//! WireGuard key management.
//!
//! WireGuard uses x25519 Diffie-Hellman keys (32 bytes each).
//! Keys are represented as base64-encoded strings in configuration
//! and as raw bytes internally.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::MeshError;

/// A WireGuard keypair (private + public).
#[derive(Clone)]
pub struct WgKeypair {
    pub private: StaticSecret,
    pub public: PublicKey,
}

impl WgKeypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let private = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&private);
        Self { private, public }
    }

    /// Create a keypair from a base64-encoded private key.
    pub fn from_private_base64(b64: &str) -> Result<Self, MeshError> {
        let bytes = BASE64
            .decode(b64)
            .map_err(|e| MeshError::InvalidKey(format!("bad base64: {e}")))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| MeshError::InvalidKey("private key must be 32 bytes".into()))?;
        let private = StaticSecret::from(bytes);
        let public = PublicKey::from(&private);
        Ok(Self { private, public })
    }

    /// Encode the private key as base64.
    pub fn private_base64(&self) -> String {
        BASE64.encode(self.private.to_bytes())
    }

    /// Encode the public key as base64.
    pub fn public_base64(&self) -> String {
        BASE64.encode(self.public.as_bytes())
    }
}

impl std::fmt::Debug for WgKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgKeypair")
            .field("public", &self.public_base64())
            .field("private", &"[redacted]")
            .finish()
    }
}

/// A serializable public key (base64-encoded).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WgPublicKey(pub String);

impl WgPublicKey {
    /// Create from raw public key bytes.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(BASE64.encode(bytes))
    }

    /// Create from a PublicKey.
    pub fn from_public(key: &PublicKey) -> Self {
        Self::from_bytes(key.as_bytes())
    }

    /// Decode to raw bytes.
    pub fn to_bytes(&self) -> Result<[u8; 32], MeshError> {
        let bytes = BASE64
            .decode(&self.0)
            .map_err(|e| MeshError::InvalidKey(format!("bad base64: {e}")))?;
        bytes
            .try_into()
            .map_err(|_| MeshError::InvalidKey("public key must be 32 bytes".into()))
    }

    /// Convert to an x25519 PublicKey.
    pub fn to_public_key(&self) -> Result<PublicKey, MeshError> {
        let bytes = self.to_bytes()?;
        Ok(PublicKey::from(bytes))
    }
}

impl std::fmt::Display for WgPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_keypair() {
        let kp = WgKeypair::generate();
        assert_eq!(kp.private_base64().len(), 44); // 32 bytes -> 44 base64 chars
        assert_eq!(kp.public_base64().len(), 44);
    }

    #[test]
    fn roundtrip_private_key() {
        let kp1 = WgKeypair::generate();
        let b64 = kp1.private_base64();
        let kp2 = WgKeypair::from_private_base64(&b64).unwrap();
        assert_eq!(kp1.public_base64(), kp2.public_base64());
    }

    #[test]
    fn public_key_serialization() {
        let kp = WgKeypair::generate();
        let pk = WgPublicKey::from_public(&kp.public);
        let bytes = pk.to_bytes().unwrap();
        assert_eq!(bytes, *kp.public.as_bytes());

        let json = serde_json::to_string(&pk).unwrap();
        let pk2: WgPublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(pk, pk2);
    }

    #[test]
    fn different_keypairs_different_keys() {
        let kp1 = WgKeypair::generate();
        let kp2 = WgKeypair::generate();
        assert_ne!(kp1.public_base64(), kp2.public_base64());
    }

    #[test]
    fn invalid_base64_rejected() {
        assert!(WgKeypair::from_private_base64("not-valid-base64!!!").is_err());
    }

    #[test]
    fn wrong_length_rejected() {
        let short = BASE64.encode(b"too short");
        assert!(WgKeypair::from_private_base64(&short).is_err());
    }
}
