//! File system watching using the notify crate.

#[cfg(feature = "watch")]
mod file_watcher {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
    use tokio::sync::{broadcast, RwLock};

    use crate::error::RemoteError;
    use crate::fs::security::FsPolicy;
    use crate::protocol::WatchEventKind;

    /// Manages file system watchers and broadcasts change events.
    pub struct FileWatchManager {
        policy: Arc<FsPolicy>,
        watchers: Arc<RwLock<HashMap<String, WatcherState>>>,
    }

    struct WatcherState {
        _watcher: RecommendedWatcher,
        tx: broadcast::Sender<WatchEvent>,
    }

    #[derive(Debug, Clone)]
    pub struct WatchEvent {
        pub path: String,
        pub kind: WatchEventKind,
    }

    impl FileWatchManager {
        pub fn new(policy: Arc<FsPolicy>) -> Self {
            Self {
                policy,
                watchers: Arc::new(RwLock::new(HashMap::new())),
            }
        }

        /// Start watching a path. Returns a receiver for events.
        pub async fn watch(
            &self,
            path: &str,
        ) -> Result<broadcast::Receiver<WatchEvent>, RemoteError> {
            let resolved = self.policy.resolve_path(path)?;
            let key = resolved.to_string_lossy().to_string();

            // Check if already watching
            {
                let watchers = self.watchers.read().await;
                if let Some(state) = watchers.get(&key) {
                    return Ok(state.tx.subscribe());
                }
            }

            let (tx, rx) = broadcast::channel(256);
            let tx_clone = tx.clone();

            let mut watcher = notify::recommended_watcher(move |result: Result<Event, _>| {
                if let Ok(event) = result {
                    let kind = match event.kind {
                        notify::EventKind::Create(_) => WatchEventKind::Created,
                        notify::EventKind::Modify(_) => WatchEventKind::Modified,
                        notify::EventKind::Remove(_) => WatchEventKind::Deleted,
                        _ => return,
                    };

                    for path in event.paths {
                        let _ = tx_clone.send(WatchEvent {
                            path: path.to_string_lossy().to_string(),
                            kind: kind.clone(),
                        });
                    }
                }
            })
            .map_err(|e| RemoteError::Internal(format!("Failed to create watcher: {e}")))?;

            watcher
                .watch(&resolved, RecursiveMode::Recursive)
                .map_err(|e| RemoteError::Internal(format!("Failed to watch path: {e}")))?;

            self.watchers.write().await.insert(
                key,
                WatcherState {
                    _watcher: watcher,
                    tx,
                },
            );

            Ok(rx)
        }

        /// Stop watching a path.
        pub async fn unwatch(&self, path: &str) -> Result<(), RemoteError> {
            let resolved = self.policy.resolve_path(path)?;
            let key = resolved.to_string_lossy().to_string();
            self.watchers.write().await.remove(&key);
            Ok(())
        }
    }
}

#[cfg(feature = "watch")]
pub use file_watcher::{FileWatchManager, WatchEvent};
