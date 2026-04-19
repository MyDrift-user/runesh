//! Axum WebSocket handlers for remote file explorer and CLI.
//!
//! # Usage
//!
//! ```ignore
//! use axum::{Router, routing::get};
//! use runesh_remote::handlers::{ws_remote_handler, RemoteState};
//!
//! let state = RemoteState::new(Default::default(), Default::default());
//! let app = Router::new()
//!     .route("/ws/remote", get(ws_remote_handler))
//!     .with_state(state);
//! ```

#[cfg(feature = "axum")]
mod axum_handlers {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use futures_util::{SinkExt, StreamExt};

    use crate::cli::AuditLogger;
    use crate::error::RemoteError;
    use crate::fs::security::FsPolicy;
    use crate::protocol::*;

    /// Shared state for remote WebSocket handlers.
    #[derive(Clone)]
    pub struct RemoteState {
        pub fs_policy: Arc<FsPolicy>,
        pub upload_manager: Arc<crate::fs::UploadManager>,
        pub audit: Arc<AuditLogger>,
        #[cfg(feature = "cli")]
        pub session_manager: Arc<crate::cli::SessionManager>,
    }

    impl RemoteState {
        pub fn new(
            fs_policy: FsPolicy,
            _session_config: crate::cli::session::SessionConfig,
        ) -> Self {
            let fs_policy = Arc::new(fs_policy);
            let audit = Arc::new(AuditLogger::new());

            Self {
                upload_manager: Arc::new(crate::fs::UploadManager::new(fs_policy.clone())),
                #[cfg(feature = "cli")]
                session_manager: Arc::new(crate::cli::SessionManager::new(
                    _session_config,
                    audit.clone(),
                )),
                fs_policy,
                audit,
            }
        }

        /// Create with a file-based audit logger.
        pub fn with_audit_file(
            fs_policy: FsPolicy,
            session_config: crate::cli::session::SessionConfig,
            audit_path: std::path::PathBuf,
        ) -> Self {
            let fs_policy = Arc::new(fs_policy);
            let audit = Arc::new(AuditLogger::with_file(audit_path));

            Self {
                upload_manager: Arc::new(crate::fs::UploadManager::new(fs_policy.clone())),
                #[cfg(feature = "cli")]
                session_manager: Arc::new(crate::cli::SessionManager::new(
                    session_config,
                    audit.clone(),
                )),
                fs_policy,
                audit,
            }
        }
    }

    /// WebSocket upgrade handler for the unified remote protocol.
    pub async fn ws_remote_handler(
        ws: WebSocketUpgrade,
        State(state): State<RemoteState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| handle_remote_ws(socket, state))
    }

    /// Main WebSocket loop: routes messages to fs or cli handlers.
    async fn handle_remote_ws(socket: WebSocket, state: RemoteState) {
        let (mut ws_tx, mut ws_rx) = socket.split();

        while let Some(msg) = ws_rx.next().await {
            let msg = match msg {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_)) | Err(_) => break,
                _ => continue,
            };

            let response = match serde_json::from_str::<WsMessage>(&msg) {
                Ok(WsMessage::Fs { payload }) => handle_fs_message(&state, payload).await,
                Ok(WsMessage::Cli { payload }) => handle_cli_message(&state, payload).await,
                Err(e) => serde_json::to_string(&FsResponse::Error {
                    code: "parse_error".into(),
                    message: format!("Invalid message: {e}"),
                })
                .unwrap_or_default(),
            };

            if ws_tx.send(Message::Text(response.into())).await.is_err() {
                break;
            }
        }
    }

    /// Handle a file system request.
    async fn handle_fs_message(state: &RemoteState, payload: serde_json::Value) -> String {
        let request: FsRequest = match serde_json::from_value(payload) {
            Ok(req) => req,
            Err(e) => {
                return serde_json::to_string(&FsResponse::Error {
                    code: "bad_request".into(),
                    message: format!("Invalid FS request: {e}"),
                })
                .unwrap_or_default();
            }
        };

        let response = match process_fs_request(state, request).await {
            Ok(resp) => resp,
            Err(e) => FsResponse::Error {
                code: e.error_code().into(),
                message: e.to_string(),
            },
        };

        serde_json::to_string(&response).unwrap_or_default()
    }

    /// Process a single file system request.
    async fn process_fs_request(
        state: &RemoteState,
        request: FsRequest,
    ) -> Result<FsResponse, RemoteError> {
        use crate::fs::explorer;

        match request {
            FsRequest::List { path, show_hidden } => {
                state.audit.log_fs_operation("list", &path, None).await;
                let entries = explorer::list_dir(&state.fs_policy, &path, show_hidden).await?;
                Ok(FsResponse::Listing { path, entries })
            }
            FsRequest::Stat { path } => {
                let entry = explorer::stat(&state.fs_policy, &path).await?;
                Ok(FsResponse::Stat { entry })
            }
            FsRequest::Read {
                path,
                offset,
                length,
            } => {
                state.audit.log_fs_operation("read", &path, None).await;
                let (data, total_size, checksum) =
                    explorer::read_file(&state.fs_policy, &path, offset, length).await?;
                Ok(FsResponse::FileContent {
                    path,
                    data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data),
                    offset,
                    total_size,
                    checksum,
                })
            }
            FsRequest::Write {
                path, data, append, ..
            } => {
                state.audit.log_fs_operation("write", &path, None).await;
                let decoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
                        .map_err(|e| RemoteError::BadRequest(format!("Invalid base64: {e}")))?;
                let bytes_written =
                    explorer::write_file(&state.fs_policy, &path, &decoded, append).await?;
                Ok(FsResponse::WriteOk {
                    path,
                    bytes_written,
                })
            }
            FsRequest::Mkdir { path, recursive } => {
                state.audit.log_fs_operation("mkdir", &path, None).await;
                explorer::mkdir(&state.fs_policy, &path, recursive).await?;
                Ok(FsResponse::Ok {
                    message: format!("Directory created: {path}"),
                })
            }
            FsRequest::Delete { path, recursive } => {
                state.audit.log_fs_operation("delete", &path, None).await;
                explorer::delete(&state.fs_policy, &path, recursive).await?;
                Ok(FsResponse::Ok {
                    message: format!("Deleted: {path}"),
                })
            }
            FsRequest::Copy { src, dst } => {
                state
                    .audit
                    .log_fs_operation("copy", &format!("{src} -> {dst}"), None)
                    .await;
                explorer::copy(&state.fs_policy, &src, &dst).await?;
                Ok(FsResponse::Ok {
                    message: format!("Copied {src} to {dst}"),
                })
            }
            FsRequest::Move { src, dst } => {
                state
                    .audit
                    .log_fs_operation("move", &format!("{src} -> {dst}"), None)
                    .await;
                explorer::rename(&state.fs_policy, &src, &dst).await?;
                Ok(FsResponse::Ok {
                    message: format!("Moved {src} to {dst}"),
                })
            }
            FsRequest::Search {
                path,
                pattern,
                max_results,
            } => {
                let matches =
                    explorer::search(&state.fs_policy, &path, &pattern, max_results).await?;
                Ok(FsResponse::SearchResults {
                    path,
                    pattern,
                    matches,
                })
            }
            FsRequest::Upload {
                path,
                chunk_index,
                total_chunks,
                data,
            } => {
                let decoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
                        .map_err(|e| RemoteError::BadRequest(format!("Invalid base64: {e}")))?;
                let (is_complete, percent) = state
                    .upload_manager
                    .handle_chunk(&path, chunk_index, total_chunks, &decoded)
                    .await?;
                if is_complete {
                    state
                        .audit
                        .log_fs_operation("upload_complete", &path, None)
                        .await;
                }
                Ok(FsResponse::Progress {
                    operation: "upload".into(),
                    path,
                    percent,
                })
            }
            FsRequest::Download { path } => {
                state.audit.log_fs_operation("download", &path, None).await;
                let (data, total_size, checksum) =
                    explorer::read_file(&state.fs_policy, &path, 0, 0).await?;
                Ok(FsResponse::FileContent {
                    path,
                    data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data),
                    offset: 0,
                    total_size,
                    checksum,
                })
            }
            FsRequest::Archive { paths, .. } => {
                let zip_path =
                    crate::fs::archive::create_zip_archive(&state.fs_policy, &paths).await?;
                let data = tokio::fs::read(&zip_path).await?;
                let _ = tokio::fs::remove_file(&zip_path).await;
                Ok(FsResponse::FileContent {
                    path: "archive.zip".into(),
                    data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data),
                    offset: 0,
                    total_size: data.len() as u64,
                    checksum: String::new(),
                })
            }
            #[cfg(feature = "watch")]
            FsRequest::Watch { path } | FsRequest::Unwatch { path } => Ok(FsResponse::Ok {
                message: format!("Watch operation on {path}"),
            }),
        }
    }

    /// Handle a CLI request.
    async fn handle_cli_message(state: &RemoteState, payload: serde_json::Value) -> String {
        #[cfg(not(feature = "cli"))]
        {
            let _ = (state, payload);
            return serde_json::to_string(&CliResponse::Error {
                code: "not_available".into(),
                message: "CLI feature not enabled".into(),
            })
            .unwrap_or_default();
        }

        #[cfg(feature = "cli")]
        {
            let request: CliRequest = match serde_json::from_value(payload) {
                Ok(req) => req,
                Err(e) => {
                    return serde_json::to_string(&CliResponse::Error {
                        code: "bad_request".into(),
                        message: format!("Invalid CLI request: {e}"),
                    })
                    .unwrap_or_default();
                }
            };

            let response = match process_cli_request(state, request).await {
                Ok(resp) => resp,
                Err(e) => CliResponse::Error {
                    code: e.error_code().into(),
                    message: e.to_string(),
                },
            };

            serde_json::to_string(&response).unwrap_or_default()
        }
    }

    /// Process a single CLI request.
    #[cfg(feature = "cli")]
    async fn process_cli_request(
        state: &RemoteState,
        request: CliRequest,
    ) -> Result<CliResponse, RemoteError> {
        match request {
            CliRequest::Open {
                shell,
                cols,
                rows,
                env,
                cwd,
            } => {
                let (session_id, shell_name) = state
                    .session_manager
                    .open(shell.as_deref(), cols, rows, cwd.as_deref(), &env, None)
                    .await?;
                Ok(CliResponse::Opened {
                    session_id,
                    shell: shell_name,
                })
            }
            CliRequest::Input { session_id, data } => {
                let decoded =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
                        .map_err(|e| RemoteError::BadRequest(format!("Invalid base64: {e}")))?;
                state.session_manager.input(&session_id, &decoded).await?;
                Ok(CliResponse::Sessions {
                    sessions: Vec::new(),
                })
            }
            CliRequest::Resize {
                session_id,
                cols,
                rows,
            } => {
                state
                    .session_manager
                    .resize(&session_id, cols, rows)
                    .await?;
                Ok(CliResponse::Sessions {
                    sessions: Vec::new(),
                })
            }
            CliRequest::Close { session_id } => {
                let exit_code = state.session_manager.close(&session_id, None).await?;
                Ok(CliResponse::Closed {
                    session_id,
                    exit_code,
                })
            }
            CliRequest::ListSessions => {
                let sessions = state.session_manager.list_sessions().await;
                Ok(CliResponse::Sessions { sessions })
            }
        }
    }
}

#[cfg(feature = "axum")]
pub use axum_handlers::*;
