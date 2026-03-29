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

// ── Redis-backed broadcast registry ────────────────────────────────────────

#[cfg(feature = "redis")]
mod redis_broadcast {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{broadcast, RwLock};

    /// A broadcast registry backed by Redis pub/sub for cross-pod delivery.
    ///
    /// Each pod maintains local tokio broadcast channels. When a message is sent:
    /// 1. It's delivered to local subscribers immediately
    /// 2. It's PUBLISH'd to a Redis channel so other pods receive it
    ///
    /// When subscribing to a room, a background task is spawned that SUBSCRIBE's
    /// to the Redis channel and relays messages into the local broadcast channel.
    ///
    /// ```ignore
    /// let pool = runesh_core::redis::create_redis_pool(None).unwrap();
    /// let registry = RedisBroadcastRegistry::new(pool, 128);
    ///
    /// let mut rx = registry.subscribe("room:123").await;
    /// registry.broadcast("room:123", "hello".to_string()).await;
    /// ```
    #[derive(Clone)]
    pub struct RedisBroadcastRegistry {
        pool: deadpool_redis::Pool,
        /// URL for creating dedicated subscription connections (not from pool).
        redis_url: String,
        channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
        capacity: usize,
    }

    impl RedisBroadcastRegistry {
        /// Create a new Redis-backed broadcast registry.
        ///
        /// - `pool`: deadpool-redis pool for PUBLISH commands
        /// - `capacity`: local tokio broadcast channel capacity
        pub fn new(pool: deadpool_redis::Pool, capacity: usize) -> Self {
            let redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            Self {
                pool,
                redis_url,
                channels: Arc::new(RwLock::new(HashMap::new())),
                capacity,
            }
        }

        /// Create with an explicit Redis URL (for the subscription connection).
        pub fn with_url(pool: deadpool_redis::Pool, capacity: usize, redis_url: &str) -> Self {
            Self {
                pool,
                redis_url: redis_url.to_string(),
                channels: Arc::new(RwLock::new(HashMap::new())),
                capacity,
            }
        }

        fn redis_channel(room: &str) -> String {
            format!("ws:room:{room}")
        }

        /// Get or create a local broadcast channel for a room.
        /// If the channel is new, spawns a Redis SUBSCRIBE task to relay messages.
        async fn ensure_channel(&self, room: &str) -> broadcast::Sender<String> {
            // Fast path: channel already exists
            {
                let channels = self.channels.read().await;
                if let Some(tx) = channels.get(room) {
                    return tx.clone();
                }
            }

            // Slow path: create channel and spawn subscriber
            let mut channels = self.channels.write().await;
            // Double-check after acquiring write lock
            if let Some(tx) = channels.get(room) {
                return tx.clone();
            }

            let (tx, _) = broadcast::channel(self.capacity);
            channels.insert(room.to_string(), tx.clone());

            // Spawn a background task with a dedicated connection for SUBSCRIBE
            let redis_url = self.redis_url.clone();
            let redis_channel = Self::redis_channel(room);
            let tx_clone = tx.clone();
            let channels_ref = self.channels.clone();
            let room_owned = room.to_string();

            tokio::spawn(async move {
                if let Err(e) =
                    Self::subscribe_loop(&redis_url, &redis_channel, &tx_clone).await
                {
                    tracing::error!(
                        room = %room_owned,
                        error = %e,
                        "Redis subscription task failed"
                    );
                }
                // Clean up the channel entry when the subscriber exits
                channels_ref.write().await.remove(&room_owned);
            });

            tx
        }

        /// Long-running Redis SUBSCRIBE loop. Uses a dedicated connection (not pooled)
        /// because SUBSCRIBE blocks the connection.
        async fn subscribe_loop(
            redis_url: &str,
            channel: &str,
            tx: &broadcast::Sender<String>,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let client = deadpool_redis::redis::Client::open(redis_url)?;
            let mut pubsub = client.get_async_pubsub().await?;
            pubsub.subscribe(channel).await?;

            tracing::debug!(channel = %channel, "Redis subscription started");

            loop {
                let msg: Option<deadpool_redis::redis::Msg> =
                    pubsub.on_message().next().await;

                match msg {
                    Some(msg) => {
                        let payload: String = match msg.get_payload() {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to decode Redis message");
                                continue;
                            }
                        };
                        // Relay to local broadcast; ignore if no receivers
                        let _ = tx.send(payload);

                        // If no local receivers remain, exit the loop to free the connection
                        if tx.receiver_count() == 0 {
                            tracing::debug!(
                                channel = %channel,
                                "No local receivers, stopping Redis subscription"
                            );
                            break;
                        }
                    }
                    None => {
                        tracing::warn!(channel = %channel, "Redis subscription stream ended");
                        break;
                    }
                }
            }

            Ok(())
        }

        /// Subscribe to a room. Returns a local broadcast receiver.
        /// Spawns a Redis SUBSCRIBE task if this is the first subscriber for the room.
        pub async fn subscribe(&self, room: &str) -> broadcast::Receiver<String> {
            self.ensure_channel(room).await.subscribe()
        }

        /// Broadcast a message to a room.
        /// Sends to local subscribers AND publishes to Redis for cross-pod delivery.
        pub async fn broadcast(&self, room: &str, message: String) -> usize {
            // Send to local subscribers
            let local_count = {
                let channels = self.channels.read().await;
                if let Some(tx) = channels.get(room) {
                    tx.send(message.clone()).unwrap_or(0)
                } else {
                    0
                }
            };

            // Publish to Redis for other pods
            let redis_channel = Self::redis_channel(room);
            match self.pool.get().await {
                Ok(mut conn) => {
                    let result: Result<(), _> = deadpool_redis::redis::cmd("PUBLISH")
                        .arg(&redis_channel)
                        .arg(&message)
                        .query_async(&mut *conn)
                        .await;

                    if let Err(e) = result {
                        tracing::error!(
                            room = %room,
                            error = %e,
                            "Failed to PUBLISH to Redis"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        room = %room,
                        error = %e,
                        "Failed to get Redis connection for PUBLISH"
                    );
                }
            }

            local_count
        }

        /// Clean up local channels with no active subscribers.
        pub async fn cleanup(&self) {
            let mut channels = self.channels.write().await;
            channels.retain(|_, tx| tx.receiver_count() > 0);
        }
    }

    // We need `futures_util::StreamExt` for the `next()` call on pubsub
    use futures_util::StreamExt;
}

#[cfg(feature = "redis")]
pub use redis_broadcast::RedisBroadcastRegistry;

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
/// Configuration for WebSocket connection limits.
pub struct WsLimits {
    /// Maximum message size in bytes (default: 65536 = 64 KB).
    pub max_message_size: usize,
    /// Maximum client messages per second (default: 10).
    pub max_messages_per_sec: u32,
    /// Idle timeout in seconds -- close connection after no activity (default: 300 = 5 min).
    pub idle_timeout_secs: u64,
}

impl Default for WsLimits {
    fn default() -> Self {
        Self {
            max_message_size: 65_536,
            max_messages_per_sec: 10,
            idle_timeout_secs: 300,
        }
    }
}

#[cfg(feature = "axum")]
pub async fn ws_broadcast_loop(
    socket: axum::extract::ws::WebSocket,
    registry: &BroadcastRegistry,
    room: &str,
    on_client_message: impl Fn(String),
) {
    ws_broadcast_loop_with_limits(socket, registry, room, on_client_message, WsLimits::default()).await;
}

/// WebSocket broadcast loop with configurable message size, rate, and idle limits.
#[cfg(feature = "axum")]
pub async fn ws_broadcast_loop_with_limits(
    socket: axum::extract::ws::WebSocket,
    registry: &BroadcastRegistry,
    room: &str,
    on_client_message: impl Fn(String),
    limits: WsLimits,
) {
    use futures_util::{SinkExt, StreamExt};
    use std::time::Instant;

    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut broadcast_rx = registry.subscribe(room).await;

    let mut msg_count: u32 = 0;
    let mut rate_window_start = Instant::now();
    let idle_timeout = tokio::time::Duration::from_secs(limits.idle_timeout_secs);

    loop {
        tokio::select! {
            msg = broadcast_rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_tx.send(axum::extract::ws::Message::Text(text.into())).await.is_err() {
                            break;
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
                        // Message size limit
                        if text.len() > limits.max_message_size {
                            tracing::warn!(room = %room, size = text.len(), "WebSocket message too large, dropping");
                            continue;
                        }

                        // Rate limiting (sliding window per second)
                        let now = Instant::now();
                        if now.duration_since(rate_window_start).as_secs() >= 1 {
                            msg_count = 0;
                            rate_window_start = now;
                        }
                        msg_count += 1;
                        if msg_count > limits.max_messages_per_sec {
                            tracing::warn!(room = %room, "WebSocket rate limit exceeded, dropping message");
                            continue;
                        }

                        on_client_message(text.to_string());
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(idle_timeout) => {
                tracing::debug!(room = %room, "WebSocket idle timeout, closing");
                break;
            }
        }
    }
}
