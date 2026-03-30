//! Remote file system module: secure file explorer over WebSocket.

pub mod security;
pub mod explorer;
pub mod transfer;
pub mod archive;
pub mod watch;

pub use security::FsPolicy;
pub use transfer::UploadManager;

#[cfg(feature = "watch")]
pub use watch::FileWatchManager;
