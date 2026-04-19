//! DERP relay server.
//!
//! Accepts TCP connections from mesh clients, authenticates them by
//! their WireGuard public key, and forwards encrypted packets between
//! peers. The relay never decrypts the traffic.

use std::sync::Arc;

use base64::Engine;
use bytes::BytesMut;
use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::RelayError;
use crate::frame::{self, Frame, KEY_LEN};

/// A connected client identified by their WireGuard public key.
struct Client {
    /// Channel to send frames to this client's write task.
    tx: mpsc::Sender<Frame>,
    /// Whether this client has marked us as their preferred relay.
    preferred: bool,
}

/// DERP relay server configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Address to bind (e.g., "0.0.0.0:3340").
    pub bind_addr: String,
    /// Server's public key (for identification, not crypto).
    pub server_key: [u8; KEY_LEN],
    /// Maximum clients.
    pub max_clients: usize,
    /// Per-client send buffer size.
    pub client_buffer: usize,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:3340".into(),
            server_key: [0u8; KEY_LEN],
            max_clients: 10_000,
            client_buffer: 256,
        }
    }
}

/// Shared state for the relay server.
pub struct RelayServer {
    config: RelayConfig,
    /// Connected clients indexed by their public key.
    clients: Arc<DashMap<[u8; KEY_LEN], Client>>,
    /// Watchers: clients that want peer connect/disconnect events.
    watchers: Arc<DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>>,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            clients: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
        }
    }

    /// Run the relay server, listening for connections.
    pub async fn run(&self) -> Result<(), RelayError> {
        let listener = TcpListener::bind(&self.config.bind_addr).await?;
        tracing::info!(addr = %self.config.bind_addr, "DERP relay listening");

        loop {
            let (stream, addr) = listener.accept().await?;
            tracing::debug!(%addr, "new connection");

            let clients = Arc::clone(&self.clients);
            let watchers = Arc::clone(&self.watchers);
            let server_key = self.config.server_key;
            let client_buffer = self.config.client_buffer;
            let max_clients = self.config.max_clients;

            tokio::spawn(async move {
                if let Err(e) = handle_client(
                    stream,
                    server_key,
                    clients,
                    watchers,
                    client_buffer,
                    max_clients,
                )
                .await
                {
                    tracing::debug!(%addr, error = %e, "client disconnected");
                }
            });
        }
    }

    /// Number of connected clients.
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

async fn handle_client(
    mut stream: TcpStream,
    server_key: [u8; KEY_LEN],
    clients: Arc<DashMap<[u8; KEY_LEN], Client>>,
    watchers: Arc<DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>>,
    client_buffer: usize,
    max_clients: usize,
) -> Result<(), RelayError> {
    // Step 1: Send our server key
    let mut write_buf = BytesMut::new();
    frame::encode_frame(&Frame::ServerKey { key: server_key }, &mut write_buf);
    stream.write_all(&write_buf).await?;
    write_buf.clear();

    // Step 2: Read client info to get their public key
    let mut read_buf = BytesMut::with_capacity(4096);
    let client_key = loop {
        let n = stream.read_buf(&mut read_buf).await?;
        if n == 0 {
            return Err(RelayError::Disconnected);
        }
        if let Some(frame) = frame::decode_frame(&mut read_buf)? {
            match frame {
                Frame::ClientInfo { data } => {
                    // Client info contains a JSON object with the client's public key.
                    // For now, extract the first 32 bytes as the key.
                    // In the full protocol, this is a Noise-authenticated message.
                    if data.len() < KEY_LEN {
                        return Err(RelayError::InvalidFrame(
                            "ClientInfo too short for key".into(),
                        ));
                    }
                    let mut key = [0u8; KEY_LEN];
                    key.copy_from_slice(&data[..KEY_LEN]);
                    break key;
                }
                _ => {
                    return Err(RelayError::InvalidFrame(
                        "expected ClientInfo as first frame".into(),
                    ));
                }
            }
        }
    };

    if clients.len() >= max_clients {
        tracing::warn!("max clients reached, rejecting");
        return Err(RelayError::InvalidFrame("server full".into()));
    }

    // Step 3: Set up client channels
    let (tx, mut rx) = mpsc::channel::<Frame>(client_buffer);
    clients.insert(
        client_key,
        Client {
            tx: tx.clone(),
            preferred: false,
        },
    );

    // Notify watchers
    for watcher in watchers.iter() {
        let _ = watcher
            .value()
            .try_send(Frame::PeerPresent { key: client_key });
    }

    tracing::debug!(
        key = base64::engine::general_purpose::STANDARD.encode(client_key),
        "client authenticated"
    );

    // Step 4: Split into read/write tasks
    let (mut reader, mut writer) = stream.into_split();

    // Write task: send frames from the channel to the client
    let write_clients = Arc::clone(&clients);
    let write_watchers = Arc::clone(&watchers);
    let write_handle = tokio::spawn(async move {
        let mut buf = BytesMut::with_capacity(4096);
        while let Some(frame) = rx.recv().await {
            buf.clear();
            frame::encode_frame(&frame, &mut buf);
            if writer.write_all(&buf).await.is_err() {
                break;
            }
        }
        // Cleanup on disconnect
        write_clients.remove(&client_key);
        for watcher in write_watchers.iter() {
            let _ = watcher
                .value()
                .try_send(Frame::PeerGone { key: client_key });
        }
    });

    // Read task: read frames from the client and route them
    let result = read_loop(&mut reader, &mut read_buf, client_key, &clients, &watchers).await;

    // Signal the write task to stop
    drop(tx);
    let _ = write_handle.await;

    // Ensure cleanup if read_loop exits first
    clients.remove(&client_key);
    for watcher in watchers.iter() {
        let _ = watcher
            .value()
            .try_send(Frame::PeerGone { key: client_key });
    }

    result
}

async fn read_loop(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    buf: &mut BytesMut,
    client_key: [u8; KEY_LEN],
    clients: &DashMap<[u8; KEY_LEN], Client>,
    watchers: &DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>,
) -> Result<(), RelayError> {
    loop {
        let n = reader.read_buf(buf).await?;
        if n == 0 {
            return Err(RelayError::Disconnected);
        }

        while let Some(f) = frame::decode_frame(buf)? {
            match f {
                Frame::SendPacket { dst_key, data } => {
                    // Forward to the destination peer
                    if let Some(peer) = clients.get(&dst_key) {
                        let recv_frame = Frame::RecvPacket {
                            src_key: client_key,
                            data,
                        };
                        // Non-blocking send; drop if peer's buffer is full
                        let _ = peer.tx.try_send(recv_frame);
                    } else {
                        tracing::trace!(
                            dst = base64::engine::general_purpose::STANDARD.encode(dst_key),
                            "packet for unknown peer, dropping"
                        );
                    }
                }
                Frame::NotePreferred { preferred } => {
                    if let Some(mut client) = clients.get_mut(&client_key) {
                        client.preferred = preferred;
                    }
                }
                Frame::WatchConns => {
                    if let Some(client) = clients.get(&client_key) {
                        watchers.insert(client_key, client.tx.clone());
                        // Send current peer list
                        for entry in clients.iter() {
                            if *entry.key() != client_key {
                                let _ =
                                    client.tx.try_send(Frame::PeerPresent { key: *entry.key() });
                            }
                        }
                    }
                }
                Frame::Ping { data } => {
                    if let Some(client) = clients.get(&client_key) {
                        let _ = client.tx.try_send(Frame::Pong { data });
                    }
                }
                Frame::KeepAlive => {
                    // No action needed
                }
                _ => {
                    tracing::trace!("ignoring unexpected frame from client");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_creates() {
        let server = RelayServer::new(RelayConfig::default());
        assert_eq!(server.client_count(), 0);
    }
}
