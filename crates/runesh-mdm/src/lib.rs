//! Mobile device management: enrollment, configuration profiles, remote actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A managed device.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentToken {
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub used_count: u32,
    #[serde(default)]
    pub auto_profiles: Vec<String>,
}

impl EnrollmentToken {
    pub fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at && self.used_count < self.max_uses
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let json = serde_json::to_string(&d).unwrap();
        let parsed: ManagedDevice = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.platform, DevicePlatform::IOS);
    }

    #[test]
    fn enrollment_token_validity() {
        let valid = EnrollmentToken {
            token: "tok".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            max_uses: 10,
            used_count: 0,
            auto_profiles: vec![],
        };
        assert!(valid.is_valid());

        let expired = EnrollmentToken {
            token: "tok".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() - chrono::Duration::hours(1),
            max_uses: 10,
            used_count: 0,
            auto_profiles: vec![],
        };
        assert!(!expired.is_valid());

        let used_up = EnrollmentToken {
            token: "tok".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            max_uses: 5,
            used_count: 5,
            auto_profiles: vec![],
        };
        assert!(!used_up.is_valid());
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
}
