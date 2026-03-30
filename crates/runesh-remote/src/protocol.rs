//! WebSocket protocol message types for file explorer and remote CLI.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── File System Protocol ──────────────────────────────────────────────────

/// Client → Server file system requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsRequest {
    /// List directory contents.
    List {
        path: String,
        #[serde(default)]
        show_hidden: bool,
    },
    /// Get file/directory metadata.
    Stat { path: String },
    /// Read file content (with optional range).
    Read {
        path: String,
        #[serde(default)]
        offset: u64,
        /// 0 = read entire file.
        #[serde(default)]
        length: u64,
    },
    /// Write data to a file.
    Write {
        path: String,
        /// Base64-encoded data.
        data: String,
        #[serde(default)]
        offset: u64,
        /// If true, append to file instead of overwrite.
        #[serde(default)]
        append: bool,
    },
    /// Create a directory.
    Mkdir {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    /// Delete a file or directory.
    Delete {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    /// Copy a file or directory.
    Copy { src: String, dst: String },
    /// Move/rename a file or directory.
    Move { src: String, dst: String },
    /// Search for files matching a pattern.
    Search {
        path: String,
        pattern: String,
        #[serde(default = "default_max_results")]
        max_results: usize,
    },
    /// Upload a file chunk.
    Upload {
        path: String,
        chunk_index: u32,
        total_chunks: u32,
        /// Base64-encoded chunk data.
        data: String,
    },
    /// Request file download.
    Download { path: String },
    /// Create a zip archive of paths.
    Archive {
        paths: Vec<String>,
        #[serde(default = "default_archive_format")]
        format: ArchiveFormat,
    },
    /// Watch a path for changes.
    #[cfg(feature = "watch")]
    Watch { path: String },
    /// Stop watching a path.
    #[cfg(feature = "watch")]
    Unwatch { path: String },
}

fn default_max_results() -> usize {
    1000
}

fn default_archive_format() -> ArchiveFormat {
    ArchiveFormat::Zip
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Zip,
    Tar,
}

/// Server → Client file system responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsResponse {
    /// Directory listing.
    Listing {
        path: String,
        entries: Vec<FileEntry>,
    },
    /// File/directory metadata.
    Stat { entry: FileEntry },
    /// File content (base64-encoded).
    FileContent {
        path: String,
        data: String,
        offset: u64,
        total_size: u64,
        checksum: String,
    },
    /// Write confirmation.
    WriteOk {
        path: String,
        bytes_written: u64,
    },
    /// Operation success.
    Ok { message: String },
    /// Progress update for long-running operations.
    Progress {
        operation: String,
        path: String,
        percent: f32,
    },
    /// Download chunk.
    DownloadChunk {
        path: String,
        chunk_index: u32,
        total_chunks: u32,
        data: String,
        total_size: u64,
    },
    /// Search results.
    SearchResults {
        path: String,
        pattern: String,
        matches: Vec<FileEntry>,
    },
    /// File system watch event.
    WatchEvent {
        path: String,
        kind: WatchEventKind,
    },
    /// Error response.
    Error { code: String, message: String },
}

/// File entry metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<String>,
    pub created: Option<String>,
    pub permissions: FilePermissions,
}

/// Cross-platform file permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePermissions {
    pub readonly: bool,
    /// Unix mode (e.g., 0o755). None on Windows.
    pub mode: Option<u32>,
    /// Windows-style attributes.
    pub hidden: bool,
    pub system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchEventKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

// ── CLI Protocol ──────────────────────────────────────────────────────────

/// Client → Server CLI requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliRequest {
    /// Open a new terminal session.
    Open {
        /// Shell to use (e.g., "bash", "powershell"). None = system default.
        shell: Option<String>,
        cols: u16,
        rows: u16,
        #[serde(default)]
        env: HashMap<String, String>,
        /// Working directory.
        cwd: Option<String>,
    },
    /// Send input to a terminal session (base64-encoded).
    Input {
        session_id: String,
        data: String,
    },
    /// Resize terminal.
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    /// Close a terminal session.
    Close { session_id: String },
    /// List active sessions.
    ListSessions,
}

/// Server → Client CLI responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliResponse {
    /// Session opened successfully.
    Opened {
        session_id: String,
        shell: String,
    },
    /// Terminal output (base64-encoded).
    Output {
        session_id: String,
        data: String,
    },
    /// Session closed.
    Closed {
        session_id: String,
        exit_code: Option<u32>,
    },
    /// Active sessions list.
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    /// Error response.
    Error {
        code: String,
        message: String,
    },
}

/// Active session metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub shell: String,
    pub created_at: String,
    pub last_activity: String,
    pub cols: u16,
    pub rows: u16,
}

// ── Unified WebSocket Message ─────────────────────────────────────────────

/// Top-level WebSocket message envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "snake_case")]
pub enum WsMessage {
    Fs { payload: serde_json::Value },
    Cli { payload: serde_json::Value },
}
