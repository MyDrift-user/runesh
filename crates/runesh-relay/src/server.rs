//! DERP relay server.
//!
//! Accepts TCP connections from mesh clients, authenticates them by
//! their WireGuard public key, and forwards encrypted packets between
//! peers. The relay never decrypts the traffic.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use bytes::BytesMut;
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretBox};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Instant, timeout};

use crate::RelayError;
use crate::frame::{self, Frame, KEY_LEN};

/// Forwarding ACL: asked before the relay routes a `SendPacket` from one
/// client to another. Return `false` to drop the packet silently (the
/// per-sender rate-limit counter is not touched by a policy drop).
///
/// The default [`AllowAllForward`] returns `true` for every pair, which
/// matches the pre-ACL behaviour for existing callers.
pub trait AllowForward: Send + Sync + 'static {
    /// Called per packet. Keep it cheap; it sits in the hot path.
    fn allow(&self, sender: &[u8; KEY_LEN], recipient: &[u8; KEY_LEN]) -> bool;
}

/// Permissive ACL that lets every peer relay to every other peer. Fine
/// on a closed network where all clients are trusted; on a shared relay
/// wire a real policy.
pub struct AllowAllForward;

impl AllowForward for AllowAllForward {
    fn allow(&self, _sender: &[u8; KEY_LEN], _recipient: &[u8; KEY_LEN]) -> bool {
        true
    }
}

type HmacSha256 = Hmac<Sha256>;

/// Size of the HMAC challenge / response nonce.
pub const CHALLENGE_LEN: usize = 32;

/// Default deadline for the ClientInfo frame after the TCP accept.
const CLIENT_INFO_TIMEOUT: Duration = Duration::from_secs(10);

/// Authentication mode for a relay.
pub enum AuthMode {
    /// No authentication. Any caller may connect. Unsafe for public relays.
    None,
    /// Client must echo the shared secret in its ClientInfo frame.
    SharedKey(SecretBox<Vec<u8>>),
    /// Server issues a 32-byte random challenge after accept; client must
    /// respond with `HMAC-SHA256(shared_secret, challenge || claimed_pubkey)`.
    HmacChallenge { shared_secret: SecretBox<Vec<u8>> },
}

impl std::fmt::Debug for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthMode::None => f.write_str("AuthMode::None"),
            AuthMode::SharedKey(_) => f.write_str("AuthMode::SharedKey(REDACTED)"),
            AuthMode::HmacChallenge { .. } => f.write_str("AuthMode::HmacChallenge(REDACTED)"),
        }
    }
}

/// Authentication configuration for the relay.
#[derive(Debug)]
pub struct RelayAuthConfig {
    pub mode: AuthMode,
}

impl Default for RelayAuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::None,
        }
    }
}

/// DERP relay server configuration.
#[derive(Debug)]
pub struct RelayConfig {
    /// Address to bind (e.g., "0.0.0.0:3340").
    pub bind_addr: String,
    /// Server's public key (for identification, not crypto).
    pub server_key: [u8; KEY_LEN],
    /// Maximum clients.
    pub max_clients: usize,
    /// Per-client send buffer size.
    pub client_buffer: usize,
    /// Authentication configuration.
    pub auth: RelayAuthConfig,
    /// Per-sender packet rate (SendPacket frames per second).
    pub per_sender_pps: u32,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:3340".into(),
            server_key: [0u8; KEY_LEN],
            max_clients: 10_000,
            client_buffer: 256,
            auth: RelayAuthConfig::default(),
            per_sender_pps: 1000,
        }
    }
}

/// A connected client identified by their WireGuard public key.
struct Client {
    /// Channel to send frames to this client's write task.
    tx: mpsc::Sender<Frame>,
    /// Whether this client has marked us as their preferred relay.
    preferred: bool,
}

/// Shared state for the relay server.
pub struct RelayServer {
    config: Arc<RelayConfig>,
    /// Connected clients indexed by their public key.
    clients: Arc<DashMap<[u8; KEY_LEN], Client>>,
    /// Watchers: clients that want peer connect/disconnect events.
    watchers: Arc<DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>>,
    /// Live connection count (incremented at accept, decremented on drop).
    conn_count: Arc<AtomicUsize>,
    /// Count of frames dropped due to per-sender rate limiting (for metrics).
    rate_limited: Arc<AtomicU64>,
    /// Policy consulted before forwarding each packet. Defaults to
    /// [`AllowAllForward`].
    acl: Arc<dyn AllowForward>,
    /// Count of frames dropped by the ACL.
    acl_dropped: Arc<AtomicU64>,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        if matches!(config.auth.mode, AuthMode::None) {
            tracing::warn!(
                "DERP relay running in UNAUTHENTICATED mode; anyone on the network \
                 can connect. Configure RelayAuthConfig::HmacChallenge for production."
            );
        }
        Self {
            config: Arc::new(config),
            clients: Arc::new(DashMap::new()),
            watchers: Arc::new(DashMap::new()),
            conn_count: Arc::new(AtomicUsize::new(0)),
            rate_limited: Arc::new(AtomicU64::new(0)),
            acl: Arc::new(AllowAllForward),
            acl_dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Install a forwarding ACL. Call this before [`Self::run`] /
    /// [`Self::run_tls`]; mid-flight changes are not supported.
    pub fn with_acl<A: AllowForward>(mut self, acl: A) -> Self {
        self.acl = Arc::new(acl);
        self
    }

    /// Number of packets the ACL has dropped since startup.
    pub fn acl_dropped_count(&self) -> u64 {
        self.acl_dropped.load(Ordering::Relaxed)
    }

    /// Run the relay server over plaintext TCP.
    pub async fn run(&self) -> Result<(), RelayError> {
        let listener = TcpListener::bind(&self.config.bind_addr).await?;
        tracing::info!(addr = %self.config.bind_addr, "DERP relay listening (plaintext)");

        loop {
            let (stream, addr) = listener.accept().await?;
            if !self.reserve_slot(&addr) {
                continue;
            }

            let config = Arc::clone(&self.config);
            let clients = Arc::clone(&self.clients);
            let watchers = Arc::clone(&self.watchers);
            let conn_count = Arc::clone(&self.conn_count);
            let rate_limited = Arc::clone(&self.rate_limited);
            let acl = Arc::clone(&self.acl);
            let acl_dropped = Arc::clone(&self.acl_dropped);

            tokio::spawn(async move {
                let _guard = ConnGuard::new(conn_count);
                if let Err(e) = handle_client(
                    stream,
                    config,
                    clients,
                    watchers,
                    rate_limited,
                    acl,
                    acl_dropped,
                )
                .await
                {
                    tracing::debug!(%addr, error = %e, "client disconnected");
                }
            });
        }
    }

    /// Run the relay server over TLS. Requires the `tls` cargo feature.
    /// The caller builds a [`tokio_rustls::TlsAcceptor`] with whatever
    /// certificate, client-auth, and protocol-version policy they need.
    #[cfg(feature = "tls")]
    pub async fn run_tls(&self, acceptor: tokio_rustls::TlsAcceptor) -> Result<(), RelayError> {
        let listener = TcpListener::bind(&self.config.bind_addr).await?;
        tracing::info!(addr = %self.config.bind_addr, "DERP relay listening (TLS)");

        loop {
            let (stream, addr) = listener.accept().await?;
            if !self.reserve_slot(&addr) {
                continue;
            }

            let acceptor = acceptor.clone();
            let config = Arc::clone(&self.config);
            let clients = Arc::clone(&self.clients);
            let watchers = Arc::clone(&self.watchers);
            let conn_count = Arc::clone(&self.conn_count);
            let rate_limited = Arc::clone(&self.rate_limited);
            let acl = Arc::clone(&self.acl);
            let acl_dropped = Arc::clone(&self.acl_dropped);

            tokio::spawn(async move {
                let _guard = ConnGuard::new(conn_count);
                let tls = match acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::debug!(%addr, error = %e, "TLS handshake failed");
                        return;
                    }
                };
                if let Err(e) = handle_client(
                    tls,
                    config,
                    clients,
                    watchers,
                    rate_limited,
                    acl,
                    acl_dropped,
                )
                .await
                {
                    tracing::debug!(%addr, error = %e, "client disconnected");
                }
            });
        }
    }

    /// Account a new incoming connection against `max_clients`. Returns
    /// `true` if the slot was reserved and the caller should spawn a
    /// handler, `false` if the server is over its cap (in which case the
    /// caller should drop the stream silently).
    fn reserve_slot(&self, addr: &std::net::SocketAddr) -> bool {
        let prev = self.conn_count.fetch_add(1, Ordering::SeqCst);
        if prev >= self.config.max_clients {
            self.conn_count.fetch_sub(1, Ordering::SeqCst);
            tracing::warn!(%addr, "rejecting connection: max_clients reached");
            return false;
        }
        tracing::debug!(%addr, "new connection");
        true
    }

    /// Number of connected clients.
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Current live connection count (pre-auth + authenticated).
    pub fn conn_count(&self) -> usize {
        self.conn_count.load(Ordering::Relaxed)
    }

    /// Count of rate-limited frames since startup.
    pub fn rate_limited_count(&self) -> u64 {
        self.rate_limited.load(Ordering::Relaxed)
    }
}

/// RAII guard that decrements the connection counter on drop.
struct ConnGuard(Arc<AtomicUsize>);

impl ConnGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        Self(counter)
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Compute the expected HMAC response for a given challenge and public key.
pub fn compute_challenge_response(
    shared_secret: &[u8],
    challenge: &[u8; CHALLENGE_LEN],
    pubkey: &[u8; KEY_LEN],
) -> [u8; 32] {
    let mut mac =
        HmacSha256::new_from_slice(shared_secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(challenge);
    mac.update(pubkey);
    let out = mac.finalize().into_bytes();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

async fn handle_client<S>(
    mut stream: S,
    config: Arc<RelayConfig>,
    clients: Arc<DashMap<[u8; KEY_LEN], Client>>,
    watchers: Arc<DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>>,
    rate_limited: Arc<AtomicU64>,
    acl: Arc<dyn AllowForward>,
    acl_dropped: Arc<AtomicU64>,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Step 1: Send our server key (and, in HmacChallenge mode, a challenge).
    let mut write_buf = BytesMut::new();
    frame::encode_frame(
        &Frame::ServerKey {
            key: config.server_key,
        },
        &mut write_buf,
    );

    let challenge: Option<[u8; CHALLENGE_LEN]> = match &config.auth.mode {
        AuthMode::HmacChallenge { .. } => {
            let mut c = [0u8; CHALLENGE_LEN];
            rand::thread_rng().fill_bytes(&mut c);
            frame::encode_frame(
                &Frame::ServerChallenge { nonce: c.to_vec() },
                &mut write_buf,
            );
            Some(c)
        }
        _ => None,
    };
    stream.write_all(&write_buf).await?;
    write_buf.clear();

    // Step 2: Read client info within the deadline.
    let mut read_buf = BytesMut::with_capacity(4096);
    let client_key = timeout(CLIENT_INFO_TIMEOUT, async {
        loop {
            let n = stream.read_buf(&mut read_buf).await?;
            if n == 0 {
                return Err(RelayError::Disconnected);
            }
            if let Some(frame) = frame::decode_frame(&mut read_buf)? {
                match frame {
                    Frame::ClientInfo { data } => return Ok::<_, RelayError>(data),
                    _ => {
                        return Err(RelayError::InvalidFrame(
                            "expected ClientInfo as first frame".into(),
                        ));
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| RelayError::HandshakeTimeout)??;

    // Validate ClientInfo and extract the client's public key.
    if client_key.len() < KEY_LEN {
        return Err(RelayError::InvalidFrame(
            "ClientInfo too short for key".into(),
        ));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&client_key[..KEY_LEN]);

    // Enforce the configured auth mode.
    match &config.auth.mode {
        AuthMode::None => {}
        AuthMode::SharedKey(secret) => {
            let expected = secret.expose_secret();
            let provided = &client_key[KEY_LEN..];
            if provided.ct_eq(expected).unwrap_u8() != 1 {
                return Err(RelayError::AuthFailed);
            }
        }
        AuthMode::HmacChallenge { shared_secret } => {
            let ch = challenge.ok_or_else(|| {
                RelayError::Protocol("missing challenge state for HmacChallenge".into())
            })?;
            let provided = &client_key[KEY_LEN..];
            if provided.len() != 32 {
                return Err(RelayError::AuthFailed);
            }
            let expected = compute_challenge_response(shared_secret.expose_secret(), &ch, &key);
            if provided.ct_eq(&expected).unwrap_u8() != 1 {
                return Err(RelayError::AuthFailed);
            }
        }
    }

    // Step 3: Set up client channels.
    let (tx, mut rx) = mpsc::channel::<Frame>(config.client_buffer);
    clients.insert(
        key,
        Client {
            tx: tx.clone(),
            preferred: false,
        },
    );

    // Notify watchers.
    for watcher in watchers.iter() {
        let _ = watcher.value().try_send(Frame::PeerPresent { key });
    }

    tracing::debug!(
        key = base64::engine::general_purpose::STANDARD.encode(key),
        "client authenticated"
    );

    // Step 4: Split into read/write tasks. tokio::io::split works on any
    // AsyncRead + AsyncWrite so this path supports both plaintext TcpStream
    // and tokio_rustls::server::TlsStream<TcpStream>.
    let client_key = key;
    let (mut reader, mut writer) = tokio::io::split(stream);

    // Per-sender token bucket for packet forwarding.
    let bucket = Arc::new(Mutex::new(TokenBucket::new(
        config.per_sender_pps,
        config.per_sender_pps,
    )));

    // Write task: send frames from the channel to the client.
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
        // Cleanup on disconnect.
        write_clients.remove(&client_key);
        for watcher in write_watchers.iter() {
            let _ = watcher
                .value()
                .try_send(Frame::PeerGone { key: client_key });
        }
    });

    // Read task: read frames from the client and route them.
    let result = read_loop(
        &mut reader,
        &mut read_buf,
        client_key,
        &clients,
        &watchers,
        &bucket,
        &rate_limited,
        &acl,
        &acl_dropped,
    )
    .await;

    // Signal the write task to stop.
    drop(tx);
    let _ = write_handle.await;

    // Ensure cleanup if read_loop exits first.
    clients.remove(&client_key);
    for watcher in watchers.iter() {
        let _ = watcher
            .value()
            .try_send(Frame::PeerGone { key: client_key });
    }

    result
}

async fn read_loop<R>(
    reader: &mut R,
    buf: &mut BytesMut,
    client_key: [u8; KEY_LEN],
    clients: &DashMap<[u8; KEY_LEN], Client>,
    watchers: &DashMap<[u8; KEY_LEN], mpsc::Sender<Frame>>,
    bucket: &Arc<Mutex<TokenBucket>>,
    rate_limited: &AtomicU64,
    acl: &Arc<dyn AllowForward>,
    acl_dropped: &AtomicU64,
) -> Result<(), RelayError>
where
    R: AsyncRead + Unpin,
{
    loop {
        let n = reader.read_buf(buf).await?;
        if n == 0 {
            return Err(RelayError::Disconnected);
        }

        while let Some(f) = frame::decode_frame(buf)? {
            match f {
                Frame::SendPacket { dst_key, data } => {
                    // Consult the forwarding ACL first. A policy drop is
                    // silent to the sender and does not count against the
                    // per-sender rate limit: we don't want a misconfigured
                    // ACL to starve a well-behaved client's packets.
                    if !acl.allow(&client_key, &dst_key) {
                        acl_dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::trace!(
                            src = base64::engine::general_purpose::STANDARD.encode(client_key),
                            dst = base64::engine::general_purpose::STANDARD.encode(dst_key),
                            "ACL dropped SendPacket"
                        );
                        continue;
                    }
                    // Apply per-sender rate limit.
                    let allowed = { bucket.lock().await.try_take() };
                    if !allowed {
                        rate_limited.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!("rate limit exceeded for sender, dropping SendPacket");
                        continue;
                    }
                    if let Some(peer) = clients.get(&dst_key) {
                        let recv_frame = Frame::RecvPacket {
                            src_key: client_key,
                            data,
                        };
                        let _ = peer.tx.try_send(recv_frame);
                    } else {
                        tracing::trace!(
                            dst = base64::engine::general_purpose::STANDARD.encode(dst_key),
                            "packet for unknown peer, dropping"
                        );
                    }
                }
                Frame::ForwardPacket { .. } => {
                    // Server-to-server only; never legal from a client.
                    return Err(RelayError::Protocol(
                        "ForwardPacket not permitted on a client connection".into(),
                    ));
                }
                Frame::NotePreferred { preferred } => {
                    if let Some(mut client) = clients.get_mut(&client_key) {
                        client.preferred = preferred;
                    }
                }
                Frame::WatchConns => {
                    if let Some(client) = clients.get(&client_key) {
                        watchers.insert(client_key, client.tx.clone());
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
                Frame::KeepAlive => {}
                _ => {
                    tracing::trace!("ignoring unexpected frame from client");
                }
            }
        }
    }
}

/// Simple token bucket for per-sender rate limiting.
struct TokenBucket {
    capacity: u32,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            capacity,
            tokens: capacity as f64,
            refill_per_sec: refill_per_sec as f64,
            last: Instant::now(),
        }
    }

    fn try_take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity as f64);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
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
        assert_eq!(server.conn_count(), 0);
    }

    #[test]
    fn allow_all_acl_permits_every_pair() {
        let acl = AllowAllForward;
        assert!(acl.allow(&[1u8; KEY_LEN], &[2u8; KEY_LEN]));
        assert!(acl.allow(&[99u8; KEY_LEN], &[99u8; KEY_LEN]));
    }

    #[test]
    fn with_acl_installs_custom_policy() {
        /// Toy ACL: deny any forward where the recipient's first byte
        /// matches a configured value.
        struct DenyFirstByte(u8);
        impl AllowForward for DenyFirstByte {
            fn allow(&self, _: &[u8; KEY_LEN], recipient: &[u8; KEY_LEN]) -> bool {
                recipient[0] != self.0
            }
        }

        let server = RelayServer::new(RelayConfig::default()).with_acl(DenyFirstByte(0xAA));
        assert!(server.acl.allow(&[1u8; KEY_LEN], &[1u8; KEY_LEN]));
        assert!(!server.acl.allow(&[1u8; KEY_LEN], &[0xAA; KEY_LEN]));
        assert_eq!(server.acl_dropped_count(), 0);
    }

    #[test]
    fn challenge_response_round_trip() {
        let secret = b"supersecret".to_vec();
        let challenge = [7u8; CHALLENGE_LEN];
        let pubkey = [42u8; KEY_LEN];
        let resp = compute_challenge_response(&secret, &challenge, &pubkey);

        // Same inputs produce the same response.
        let resp2 = compute_challenge_response(&secret, &challenge, &pubkey);
        assert_eq!(resp, resp2);

        // Different secret -> different response.
        let resp3 = compute_challenge_response(b"other", &challenge, &pubkey);
        assert_ne!(resp, resp3);

        // Different pubkey -> different response.
        let other_key = [43u8; KEY_LEN];
        let resp4 = compute_challenge_response(&secret, &challenge, &other_key);
        assert_ne!(resp, resp4);

        // Different challenge -> different response.
        let mut other_ch = challenge;
        other_ch[0] ^= 0xff;
        let resp5 = compute_challenge_response(&secret, &other_ch, &pubkey);
        assert_ne!(resp, resp5);
    }

    #[test]
    fn token_bucket_rate_limits() {
        let mut bucket = TokenBucket::new(2, 0); // no refill
        assert!(bucket.try_take());
        assert!(bucket.try_take());
        assert!(!bucket.try_take());
    }
}
