//! WebSocket broadcast pattern using tokio channels.
//!
//! Provides a registry of broadcast channels keyed by room/topic.
//! Used for real-time updates (chat, notifications, live data).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// A registry of broadcast channels keyed by room/topic string.
///
/// Usage:
/// ```ignore
/// let registry = BroadcastRegistry::new(128);
///
/// // Subscribe a WebSocket client to a room
/// let mut rx = registry.subscribe("room:123").await;
///
/// // Broadcast a message to all subscribers
/// registry.send("room:123", "hello".to_string()).await;
/// ```
#[derive(Clone)]
pub struct BroadcastRegistry {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
    capacity: usize,
}

impl BroadcastRegistry {
    /// Create a new registry with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            capacity,
        }
    }

    /// Get or create a broadcast sender for a room.
    pub async fn get_or_create(&self, room: &str) -> broadcast::Sender<String> {
        {
            let channels = self.channels.read().await;
            if let Some(tx) = channels.get(room) {
                if tx.receiver_count() > 0 {
                    return tx.clone();
                }
            }
        }

        let mut channels = self.channels.write().await;
        let tx = channels
            .entry(room.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity).0);
        tx.clone()
    }

    /// Subscribe to a room. Returns a receiver.
    pub async fn subscribe(&self, room: &str) -> broadcast::Receiver<String> {
        self.get_or_create(room).await.subscribe()
    }

    /// Send a message to all subscribers of a room.
    /// Returns the number of receivers that got the message.
    pub async fn send(&self, room: &str, message: String) -> usize {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(room) {
            tx.send(message).unwrap_or(0)
        } else {
            0
        }
    }

    /// Clean up channels with no active subscribers.
    pub async fn cleanup(&self) {
        let mut channels = self.channels.write().await;
        channels.retain(|_, tx| tx.receiver_count() > 0);
    }
}

/// Axum WebSocket handler helper. Handles upgrade and runs a loop with
/// broadcast subscription + client message handling.
///
/// Usage:
/// ```ignore
/// async fn ws_handler(
///     ws: WebSocketUpgrade,
///     State(state): State<Arc<AppState>>,
/// ) -> impl IntoResponse {
///     ws.on_upgrade(move |socket| handle_ws(socket, state))
/// }
///
/// async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
///     ws_broadcast_loop(socket, &state.broadcast, "events", |msg| {
///         // Handle client messages (optional)
///         tracing::debug!("Client sent: {}", msg);
///     }).await;
/// }
/// ```
#[cfg(feature = "axum")]
pub async fn ws_broadcast_loop(
    socket: axum::extract::ws::WebSocket,
    registry: &BroadcastRegistry,
    room: &str,
    on_client_message: impl Fn(String),
) {
    use futures_util::{SinkExt, StreamExt};

    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut broadcast_rx = registry.subscribe(room).await;

    loop {
        tokio::select! {
            msg = broadcast_rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_tx.send(axum::extract::ws::Message::Text(text.into())).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(room = %room, skipped = n, "WebSocket client lagged");
                    }
                    Err(_) => break,
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Text(text))) => {
                        on_client_message(text.to_string());
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
