//! Cross-platform virtual filesystem with cloud provider integration.
//!
//! Makes remote files appear natively in the OS file explorer (Windows Explorer,
//! macOS Finder, Linux Nautilus/Dolphin) with on-demand hydration — like OneDrive's
//! "Files On-Demand".
//!
//! # Features
//!
//! - **Cloud Provider Integration**: Files visible in Explorer without downloading
//! - **On-Demand Hydration**: Content fetched only when files are opened
//! - **Overlay Writes**: Copy-on-write layer for user-specific edits (school use case)
//! - **LRU Cache**: Automatic eviction when cache exceeds configured limit
//! - **Multi-Tenant**: Teachers maintain originals, students get personal overlays
//!
//! # Platform Support
//!
//! | Platform | Backend | Cloud Icons |
//! |----------|---------|-------------|
//! | Windows | Cloud Filter API (cfapi) | Yes (blue cloud, green check) |
//! | Linux | FUSE via `fuser` | No (standard file icons) |
//! | macOS | FUSE-T via `fuser` | No (standard file icons) |
//!
//! # Quick Start
//!
//! ```ignore
//! use runesh_vfs::{VfsConfig, WriteMode, ProviderRole, MountRegistry, CacheManager};
//! use runesh_vfs::overlay::OverlayProvider;
//! use std::sync::Arc;
//!
//! // Your file provider (HTTP, S3, database, etc.)
//! let base_provider = Arc::new(MyHttpProvider::new("https://school.example.com/cs101"));
//!
//! // Student overlay — edits go to personal space, originals untouched
//! let overlay = OverlayProvider::new(
//!     base_provider,
//!     PathBuf::from("~/.runesh-vfs/overlays/alice"),
//! ).await?;
//!
//! let config = VfsConfig {
//!     mount_point: PathBuf::from("~/Course Files/CS 101"),
//!     display_name: "CS 101".into(),
//!     write_mode: WriteMode::WriteOverlay {
//!         overlay_path: PathBuf::from("~/.runesh-vfs/overlays/alice"),
//!         sync_endpoint: None,
//!     },
//!     role: ProviderRole::Student {
//!         user_id: "alice".into(),
//!         overlay_path: PathBuf::from("~/.runesh-vfs/overlays/alice"),
//!         sync_endpoint: None,
//!     },
//!     ..Default::default()
//! };
//!
//! let cache = Arc::new(CacheManager::new(
//!     PathBuf::from("~/.cache/runesh-vfs"),
//!     1_073_741_824, // 1 GB
//! ).await?);
//!
//! let registry = MountRegistry::new();
//! registry.mount("cs101", config, Arc::new(overlay), cache).await?;
//! ```

pub mod cache;
pub mod config;
pub mod error;
pub mod overlay;
pub mod platform;
pub mod provider;
pub mod registry;

pub use cache::CacheManager;
pub use config::{ProviderRole, VfsConfig, WriteMode};
pub use error::VfsError;
pub use overlay::OverlayProvider;
pub use provider::{FileProvider, VfsEntry};
pub use registry::MountRegistry;
