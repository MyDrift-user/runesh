//! Remote file system module: secure file explorer over WebSocket.

pub mod archive;
pub mod explorer;
pub mod security;
pub mod transfer;
pub mod watch;

pub use security::FsPolicy;
pub use transfer::UploadManager;

#[cfg(feature = "watch")]
pub use watch::FileWatchManager;
