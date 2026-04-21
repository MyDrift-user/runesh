//! VFS mount configuration, write modes, and role-based access.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Platform mount options that affect security. Ownership settings are
/// explicit because the old default of `allow_other` with the process uid
/// is unsafe in multi-tenant contexts.
///
/// The field names carry a loud `_unsafe` suffix so that turning them on
/// is visible in code review and in any JSON config file. Serde aliases
/// accept the legacy `allow_other` / `allow_root` names so existing
/// configs keep loading.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MountConfig {
    /// Allow users other than the mounter to access the mount (FUSE
    /// `allow_other`). **Default false.** Only enable on a single-user
    /// workstation or a mount that is intentionally shared. Never enable
    /// on a multi-tenant host where other local uids belong to other
    /// tenants. Emits a warning at mount time.
    #[serde(default, alias = "allow_other")]
    pub allow_other_users_unsafe: bool,
    /// Allow root to access the mount (FUSE `allow_root`, requires
    /// `user_allow_other` in /etc/fuse.conf). **Default false.**
    #[serde(default, alias = "allow_root")]
    pub allow_root_unsafe: bool,
    /// Fallback uid used for entries whose provider does not supply one.
    /// When None, the process uid is used (not recommended for multi-tenant).
    #[serde(default)]
    pub default_uid: Option<u32>,
    /// Fallback gid used for entries whose provider does not supply one.
    #[serde(default)]
    pub default_gid: Option<u32>,
}

/// Configuration for a virtual filesystem mount point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VfsConfig {
    /// Where the VFS appears in the OS file explorer.
    pub mount_point: PathBuf,
    /// Display name shown in Explorer/Finder sidebar.
    pub display_name: String,
    /// Optional icon path (.ico on Windows, .icns on macOS).
    pub icon_path: Option<PathBuf>,
    /// Write mode — controls how file modifications are handled.
    pub write_mode: WriteMode,
    /// User role — determines access level and overlay behavior.
    pub role: ProviderRole,
    /// Local cache directory for hydrated files.
    pub cache_dir: PathBuf,
    /// Maximum cache size in bytes (default: 1 GB).
    #[serde(default = "default_cache_max")]
    pub cache_max_bytes: u64,
    /// Unique provider identity string (e.g., "com.school.courses").
    pub provider_id: String,
    /// Provider version string.
    pub provider_version: String,
    /// Platform mount options (FUSE `allow_other`, default uid/gid, ...).
    #[serde(default)]
    pub mount: MountConfig,
}

fn default_cache_max() -> u64 {
    1_073_741_824 // 1 GB
}

/// Write mode — how file modifications are handled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WriteMode {
    /// No writes allowed — files are read-only in the file explorer.
    ReadOnly,

    /// Writes go directly back to the FileProvider (standard cloud sync).
    WriteThrough,

    /// Writes saved to local storage only — never synced to the provider.
    WriteLocal {
        /// Where locally-modified files are stored.
        local_storage_path: PathBuf,
    },

    /// Reads from provider, writes to a separate overlay storage.
    /// Original files from provider don't consume extra storage.
    /// Only modified/new files are stored in the overlay.
    WriteOverlay {
        /// Where overlay (modified) files are stored.
        overlay_path: PathBuf,
        /// Optional: remote endpoint to sync overlay changes to.
        sync_endpoint: Option<String>,
    },
}

/// User role — determines permissions and overlay behavior.
///
/// Supports multi-tenant scenarios like schools where teachers maintain
/// originals and students get personal overlay spaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderRole {
    /// Full read-write access to the base provider (teacher/admin).
    Admin,

    /// Read from base, writes go to user-specific overlay (student).
    /// Original files are never modified. Only edits consume per-user storage.
    Student {
        /// Unique user identifier.
        user_id: String,
        /// Where this student's overlay files are stored locally.
        overlay_path: PathBuf,
        /// Optional: remote server to sync the student's overlay to.
        sync_endpoint: Option<String>,
    },

    /// Read-only access — no writes allowed anywhere (guest/viewer).
    Viewer,
}

impl ProviderRole {
    /// Whether this role allows any form of writing.
    pub fn can_write(&self) -> bool {
        matches!(self, ProviderRole::Admin | ProviderRole::Student { .. })
    }

    /// Whether writes go directly to the base provider.
    pub fn writes_to_base(&self) -> bool {
        matches!(self, ProviderRole::Admin)
    }

    /// Get the overlay path for this role (if applicable).
    pub fn overlay_path(&self) -> Option<&PathBuf> {
        match self {
            ProviderRole::Student { overlay_path, .. } => Some(overlay_path),
            _ => None,
        }
    }
}

impl Default for VfsConfig {
    fn default() -> Self {
        Self {
            mount_point: PathBuf::from("cloud-files"),
            display_name: "Cloud Files".into(),
            icon_path: None,
            write_mode: WriteMode::ReadOnly,
            role: ProviderRole::Viewer,
            cache_dir: std::env::temp_dir().join("runesh-vfs-cache"),
            cache_max_bytes: default_cache_max(),
            provider_id: "com.runesh.vfs".into(),
            provider_version: "1.0".into(),
            mount: MountConfig::default(),
        }
    }
}
