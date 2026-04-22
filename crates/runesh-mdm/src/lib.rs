#![deny(unsafe_code)]
//! Mobile device management: enrollment, configuration profiles, remote actions.

pub mod attestation;

pub use attestation::{
    ANDROID_KEY_ATTESTATION_OID, AttestationError, KeyDescription, SecurityLevel,
    parse_key_description,
};

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// A managed device.
///
/// If `attestation` is `None`, `serial_number` must not be trusted for any
/// authorization decision. Device identity is only proven once an
/// [`DeviceAttestation`] has been verified against a platform attestation
/// authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedDevice {
    pub id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub os_version: String,
    pub serial_number: Option<String>,
    pub enrollment_status: EnrollmentStatus,
    pub enrolled_at: Option<DateTime<Utc>>,
    pub last_checkin: Option<DateTime<Utc>>,
    #[serde(default)]
    pub installed_profiles: Vec<String>,
    #[serde(default)]
    pub compliance: bool,
    /// Platform-backed identity proof. If `None`, `serial_number` must not be
    /// trusted for authorization.
    #[serde(default)]
    pub attestation: Option<DeviceAttestation>,
}

/// Platform-backed identity proof attached to a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAttestation {
    pub kind: AttestationKind,
    /// Raw evidence (platform-specific: TPM quote, key attestation, DeviceCheck
    /// token, etc.).
    #[serde(with = "bytes_as_base64")]
    pub evidence: Vec<u8>,
    pub verified_at: DateTime<Utc>,
    pub verifier_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationKind {
    Tpm,
    AndroidKeyAttestation,
    AppleDeviceCheck,
    ManualCsr,
}

mod bytes_as_base64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        ser.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        use base64::Engine;
        let s = String::deserialize(de)?;
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    Windows,
    MacOS,
    Linux,
    IOS,
    Android,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentStatus {
    Pending,
    Enrolled,
    Unenrolled,
    Blocked,
}

/// A configuration profile to push to devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigProfile {
    pub id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub profile_type: ProfileType,
    /// Profile payload (platform-specific format).
    pub payload: serde_json::Value,
    /// Ed25519 signature over the canonical JSON of `{id, name, platform,
    /// profile_type, payload}`. When a verifier is configured, profiles without
    /// a valid signature must be rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_signature: Option<Vec<u8>>,
}

impl ConfigProfile {
    /// Serialize the signed portion of the profile to canonical JSON bytes.
    fn canonical_bytes(&self) -> Result<Vec<u8>, MdmError> {
        let value = serde_json::json!({
            "id": self.id,
            "name": self.name,
            "platform": self.platform,
            "profile_type": self.profile_type,
            "payload": self.payload,
        });
        serde_json::to_vec(&value)
            .map_err(|e| MdmError::ActionFailed(format!("canonical serialize: {e}")))
    }

    /// Verify the profile signature against the supplied Ed25519 public key.
    pub fn verify_profile_signature(&self, pubkey: &VerifyingKey) -> Result<(), MdmError> {
        let sig_bytes = self
            .profile_signature
            .as_deref()
            .ok_or(MdmError::UnsignedProfile)?;
        let sig_arr: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| MdmError::InvalidSignature)?;
        let sig = Signature::from_bytes(&sig_arr);
        let msg = self.canonical_bytes()?;
        pubkey
            .verify(&msg, &sig)
            .map_err(|_| MdmError::InvalidSignature)
    }

    /// Validate that the payload matches the schema for `profile_type`.
    pub fn validate_payload(&self) -> Result<(), MdmError> {
        match self.profile_type {
            ProfileType::Wifi => {
                serde_json::from_value::<WifiPayload>(self.payload.clone())
                    .map_err(|e| MdmError::InvalidPayload(format!("wifi: {e}")))?;
            }
            ProfileType::Vpn => {
                serde_json::from_value::<VpnPayload>(self.payload.clone())
                    .map_err(|e| MdmError::InvalidPayload(format!("vpn: {e}")))?;
            }
            ProfileType::Email => {
                serde_json::from_value::<EmailPayload>(self.payload.clone())
                    .map_err(|e| MdmError::InvalidPayload(format!("email: {e}")))?;
            }
            ProfileType::Passcode
            | ProfileType::Certificate
            | ProfileType::Restrictions
            | ProfileType::Custom => {
                if !self.payload.is_object() {
                    return Err(MdmError::InvalidPayload(
                        "payload must be a JSON object".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Minimal Wi-Fi payload schema.
#[derive(Debug, Clone, Deserialize)]
pub struct WifiPayload {
    pub ssid: String,
    pub security: String,
    #[serde(default)]
    pub password: Option<String>,
}

/// Minimal VPN payload schema.
#[derive(Debug, Clone, Deserialize)]
pub struct VpnPayload {
    #[serde(rename = "type")]
    pub vpn_type: String,
    #[serde(default)]
    pub server: Option<String>,
}

/// Minimal email payload schema.
#[derive(Debug, Clone, Deserialize)]
pub struct EmailPayload {
    pub address: String,
    pub incoming_host: String,
    pub outgoing_host: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileType {
    Wifi,
    Vpn,
    Email,
    Certificate,
    Restrictions,
    Passcode,
    Custom,
}

/// Remote actions that can be sent to a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAction {
    Lock,
    Wipe,
    Restart,
    Unenroll,
    InstallProfile(String),
    RemoveProfile(String),
    UpdateOs,
}

/// An enrollment token for device onboarding.
///
/// Use [`EnrollmentToken::consume`] for atomic use accounting across concurrent
/// enrollments.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollmentToken {
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    #[serde(with = "atomic_u32_as_plain")]
    pub used_count: AtomicU32,
    #[serde(default)]
    pub auto_profiles: Vec<String>,
}

mod atomic_u32_as_plain {
    use std::sync::atomic::{AtomicU32, Ordering};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &AtomicU32, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(v.load(Ordering::Relaxed))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AtomicU32, D::Error> {
        let n = u32::deserialize(d)?;
        Ok(AtomicU32::new(n))
    }
}

impl Clone for EnrollmentToken {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            max_uses: self.max_uses,
            used_count: AtomicU32::new(self.used_count.load(Ordering::Relaxed)),
            auto_profiles: self.auto_profiles.clone(),
        }
    }
}

impl EnrollmentToken {
    pub fn new(token: String, expires_at: DateTime<Utc>, max_uses: u32) -> Self {
        Self {
            token,
            created_at: Utc::now(),
            expires_at,
            max_uses,
            used_count: AtomicU32::new(0),
            auto_profiles: Vec::new(),
        }
    }

    pub fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at && self.used_count.load(Ordering::Acquire) < self.max_uses
    }

    /// Observed count of consumed uses.
    pub fn used_count(&self) -> u32 {
        self.used_count.load(Ordering::Acquire)
    }

    /// Atomically consume one use of the token. Returns `Ok(used)` where `used`
    /// is the value after increment. Fails with [`MdmError::TokenExpired`] if
    /// the token has expired or reached its use cap.
    pub fn consume(&self) -> Result<u32, MdmError> {
        if Utc::now() >= self.expires_at {
            return Err(MdmError::TokenExpired);
        }
        let max = self.max_uses;
        let next = self
            .used_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current >= max {
                    None
                } else {
                    Some(current + 1)
                }
            })
            .map_err(|_| MdmError::TokenExpired)?;
        Ok(next + 1)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MdmError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("enrollment token expired")]
    TokenExpired,
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("profile signature invalid")]
    InvalidSignature,
    #[error("profile is unsigned")]
    UnsignedProfile,
    #[error("invalid profile payload: {0}")]
    InvalidPayload(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_signing_key() -> SigningKey {
        // Deterministic test key; never reuse outside tests.
        let bytes: [u8; 32] = [7u8; 32];
        SigningKey::from_bytes(&bytes)
    }

    #[test]
    fn device_serialization() {
        let d = ManagedDevice {
            id: "d1".into(),
            name: "iPhone 15".into(),
            platform: DevicePlatform::IOS,
            os_version: "17.4".into(),
            serial_number: Some("SN123".into()),
            enrollment_status: EnrollmentStatus::Enrolled,
            enrolled_at: Some(Utc::now()),
            last_checkin: Some(Utc::now()),
            installed_profiles: vec!["wifi".into(), "vpn".into()],
            compliance: true,
            attestation: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        let parsed: ManagedDevice = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.platform, DevicePlatform::IOS);
    }

    #[test]
    fn enrollment_token_validity() {
        let valid =
            EnrollmentToken::new("tok".into(), Utc::now() + chrono::Duration::hours(24), 10);
        assert!(valid.is_valid());

        let expired =
            EnrollmentToken::new("tok".into(), Utc::now() - chrono::Duration::hours(1), 10);
        assert!(!expired.is_valid());

        let used_up =
            EnrollmentToken::new("tok".into(), Utc::now() + chrono::Duration::hours(24), 5);
        for _ in 0..5 {
            used_up.consume().unwrap();
        }
        assert!(!used_up.is_valid());
        assert!(used_up.consume().is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_consume_respects_max_uses() {
        let token = std::sync::Arc::new(EnrollmentToken::new(
            "tok".into(),
            Utc::now() + chrono::Duration::hours(1),
            100,
        ));
        let mut tasks = Vec::new();
        for _ in 0..200 {
            let t = token.clone();
            tasks.push(tokio::spawn(async move { t.consume().is_ok() }));
        }
        let mut successes = 0usize;
        for t in tasks {
            if t.await.unwrap() {
                successes += 1;
            }
        }
        assert_eq!(successes, 100);
        assert_eq!(token.used_count(), 100);
    }

    #[test]
    fn all_platforms() {
        for p in [
            DevicePlatform::Windows,
            DevicePlatform::MacOS,
            DevicePlatform::Linux,
            DevicePlatform::IOS,
            DevicePlatform::Android,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            let parsed: DevicePlatform = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, p);
        }
    }

    #[test]
    fn remote_actions() {
        let actions = vec![
            RemoteAction::Lock,
            RemoteAction::Wipe,
            RemoteAction::Restart,
            RemoteAction::InstallProfile("wifi".into()),
            RemoteAction::UpdateOs,
        ];
        for a in actions {
            let json = serde_json::to_string(&a).unwrap();
            let parsed: RemoteAction = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, a);
        }
    }

    #[test]
    fn profile_payload_validation() {
        let mut wifi = ConfigProfile {
            id: "w1".into(),
            name: "Corp Wi-Fi".into(),
            platform: DevicePlatform::IOS,
            profile_type: ProfileType::Wifi,
            payload: serde_json::json!({"ssid": "corp", "security": "wpa2"}),
            profile_signature: None,
        };
        assert!(wifi.validate_payload().is_ok());
        wifi.payload = serde_json::json!({"ssid": "corp"});
        assert!(wifi.validate_payload().is_err());
    }

    #[test]
    fn profile_signature_verifies() {
        let signing = test_signing_key();
        let verifying = signing.verifying_key();

        let mut profile = ConfigProfile {
            id: "w1".into(),
            name: "Corp Wi-Fi".into(),
            platform: DevicePlatform::IOS,
            profile_type: ProfileType::Wifi,
            payload: serde_json::json!({"ssid": "corp", "security": "wpa2"}),
            profile_signature: None,
        };
        let msg = profile.canonical_bytes().unwrap();
        let sig = signing.sign(&msg);
        profile.profile_signature = Some(sig.to_bytes().to_vec());

        assert!(profile.verify_profile_signature(&verifying).is_ok());

        // Tamper: changing payload invalidates signature.
        profile.payload = serde_json::json!({"ssid": "evil", "security": "wpa2"});
        assert!(profile.verify_profile_signature(&verifying).is_err());
    }

    #[test]
    fn unsigned_profile_rejected_by_verifier() {
        let verifying = test_signing_key().verifying_key();
        let profile = ConfigProfile {
            id: "w1".into(),
            name: "Corp Wi-Fi".into(),
            platform: DevicePlatform::IOS,
            profile_type: ProfileType::Wifi,
            payload: serde_json::json!({"ssid": "corp", "security": "wpa2"}),
            profile_signature: None,
        };
        assert!(matches!(
            profile.verify_profile_signature(&verifying),
            Err(MdmError::UnsignedProfile)
        ));
    }
}
