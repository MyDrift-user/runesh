# Helvetia Implementation Plan

Comprehensive implementation plan for AI subagents building the Helvetia platform on top of RUNESH.

## Architecture Decision Record

### Decision 1: Tailscale client compatibility via Headscale-compatible coordination server

**Choice:** Implement the TS2021 Noise handshake protocol so stock Tailscale clients can connect.

**Why:** The user wants Tailscale client support. Headscale (Go, 20K+ stars) proves this is viable. The TS2021 protocol uses Noise IK pattern over WebSocket-upgraded HTTP, running H2C inside the Noise tunnel. Official Tailscale apps for Windows/macOS/Linux/iOS/Android then work out of the box.

**Risk:** The protocol is undocumented; Headscale reverse-engineered it from the open-source Tailscale client. Protocol changes in new Tailscale client versions could break compatibility. Mitigation: pin minimum client version, test against Tailscale client releases in CI.

**Reference source code:** `github.com/juanfont/headscale` (Go, ~15K lines of protocol handling). Key files: `hscontrol/protocol_ts2021.go`, `hscontrol/noise.go`, `hscontrol/mapper/`.

### Decision 2: boringtun for WireGuard data plane

**Choice:** `boringtun` v0.7.0 (Cloudflare, BSD-3)

**Why:** Production-proven userspace WireGuard in Rust. Used by EasyTier and Firezone. Usable as a library. Actively maintained (last release 2026-01). Combined with the existing `runesh-tun` crate for the TUN device layer.

**Crate:** `boringtun = "0.7"`

### Decision 3: pingora for reverse proxy

**Choice:** `pingora` v0.8.0 (Cloudflare)

**Why:** Only actively maintained, production-proven reverse proxy framework in Rust. Powers Cloudflare's edge (millions of domains). The `ProxyHttp` trait with callbacks (`upstream_peer`, `request_filter`, `response_filter`) is purpose-built for multi-tenant routing. Supports HTTP/1.1, HTTP/2, gRPC, WebSocket, TLS termination. Pre-1.0 but API is stable between releases.

**Gap:** No native UDP forwarding. Handle UDP separately with tokio.

**Crate:** `pingora = "0.8"`, `pingora-proxy = "0.8"`, `pingora-core = "0.8"`

### Decision 4: QUIC (quinn) for agent-to-controller, WebSocket fallback

**Choice:** `quinn` v0.11.x primary, `tokio-tungstenite` fallback

**Why:** QUIC provides multiplexed streams (no head-of-line blocking), 0-RTT reconnection, connection migration across IP changes, works over UDP (better NAT traversal). WebSocket fallback for networks that block UDP. mTLS built in via rustls.

**Crates:** `quinn = "0.11"`, `rustls = "0.23"`, `rcgen = "0.13"`

### Decision 5: instant-acme + DNS-01 for certificates

**Choice:** `instant-acme` v0.7.1 with DNS-01 via Cloudflare API

**Why:** Actively maintained by the same developer as rustls. Challenge-agnostic (you implement DNS-01 yourself via Cloudflare API). DNS-01 means any node can obtain certs (no HTTP challenge routing needed). Certs distributed to proxy nodes over the WireGuard mesh.

**Crate:** `instant-acme = "0.7"`

### Decision 6: DNS failover via Cloudflare Load Balancing or Route53

**Choice:** External DNS provider with health checks ($1.50-5/month)

**Why:** Self-hosting authoritative DNS for failover is complex and fragile. Cloudflare Load Balancing ($5/mo) or Route53 health checks ($0.50/endpoint/mo) give automatic dead-VPS removal. The proxy nodes themselves do not need to run DNS.

### Decision 7: DERP relay in Rust (custom implementation)

**Choice:** Implement DERP relay protocol from Go source

**Why:** No Rust DERP implementation exists. The protocol is simple (~1500 lines of Go): 1-byte frame type + 4-byte length + payload. DERP never decrypts WireGuard traffic; it forwards opaque packets by WireGuard public key over TCP/443. This runs on the same cheap VPS nodes as the proxy.

**Reference:** `tailscale/derp/derp.go` in the Tailscale Go source.

### Decision 8: service-manager for cross-platform daemon

**Choice:** `service-manager` v0.7.x (tauri-apps org)

**Why:** Uniform install/start/stop across systemd, launchd, Windows SCM. Already used by `runesh-tauri`. Single `ServiceManager` trait.

---

## Crate Map

New crates to add to `RUNESH/crates/`:

```
crates/
  runesh-coord/        # Tailscale-compatible coordination server (TS2021 + Noise)
  runesh-mesh/         # WireGuard mesh management (peer maps, key exchange, ACLs)
  runesh-relay/        # DERP relay server (TCP/443 packet forwarding)
  runesh-proxy/        # pingora-based reverse proxy (SNI routing, ACME, multi-tenant)
  runesh-agent/        # Endpoint daemon (enrollment, heartbeat, feature composition)
  runesh-acl/          # HuJSON ACL parser, identity-based rules, dry-run preview
```

### Crate dependency graph

```
runesh-agent
  depends on: runesh-mesh, runesh-inventory, runesh-remote, runesh-desktop,
              runesh-core, runesh-auth, runesh-telemetry, runesh-tun

runesh-coord
  depends on: runesh-mesh, runesh-core, runesh-auth

runesh-proxy
  depends on: runesh-mesh, runesh-core, runesh-auth

runesh-mesh
  depends on: runesh-tun, runesh-core, runesh-acl
  external:   boringtun

runesh-relay
  depends on: runesh-core
  (standalone, minimal deps)

runesh-acl
  depends on: (none, pure logic)
```

---

## Build Sequence

### Phase 0: Foundation fixes (done)
- [x] Fix UI build (PR #23)
- [x] Fix VFS fuser API (PR #24)
- [x] CI on all platforms (PR #24)

### Phase 1: `runesh-acl` (pure logic, no I/O, test first)

The ACL engine is pure computation with no external dependencies. Build and test it in isolation before anything needs it.

**What it does:**
- Parse HuJSON ACL documents (Tailscale-compatible syntax)
- Evaluate rules: given (source identity, dest identity, port) return allow/deny
- Groups from identity provider mapped to ACL groups
- Dry-run preview: diff two ACL versions, report which sessions would be affected

**Key types:**
```rust
pub struct AclPolicy { /* parsed HuJSON document */ }
pub struct AclRule { src: Vec<AclTarget>, dst: Vec<AclTarget>, ports: Vec<PortRange> }
pub enum AclTarget { Group(String), User(String), Tag(String), Cidr(IpNet), Any }
pub struct AclEvalResult { allowed: bool, matching_rule: Option<usize> }

impl AclPolicy {
    pub fn from_hujson(input: &str) -> Result<Self, AclError>;
    pub fn evaluate(&self, src: &Identity, dst: &Peer, port: u16) -> AclEvalResult;
    pub fn diff(old: &Self, new: &Self, active_sessions: &[Session]) -> Vec<AclDiffEntry>;
}
```

**Dependencies:** `serde`, `serde_json`, `ipnet`, `thiserror`

**How to get latest info:** HuJSON spec at `github.com/tailscale/hujson`. Tailscale ACL format at `tailscale.com/kb/1337/acl-syntax`. Parse with `serde_json` after stripping comments (HuJSON is JSON + comments + trailing commas).

**Tests:** Pure unit tests. No network, no async.

### Phase 2: `runesh-mesh` (WireGuard peer management)

The mesh layer manages WireGuard tunnels, peer maps, and key lifecycle. It does NOT implement the Tailscale coordination protocol (that's `runesh-coord`). It provides the building blocks.

**What it does:**
- Generate and manage WireGuard keypairs (x25519)
- Maintain a peer map: which peers exist, their endpoints, allowed IPs
- Configure boringtun tunnels via `runesh-tun`
- Apply ACL rules from `runesh-acl` to filter traffic
- Allocate mesh IPs from CGNAT range (100.64.0.0/10) per tenant

**Key types:**
```rust
pub struct MeshNode {
    pub public_key: x25519::PublicKey,
    pub private_key: x25519::StaticSecret,
    pub mesh_ip: Ipv4Addr,
    pub endpoints: Vec<SocketAddr>,
    pub allowed_ips: Vec<IpNet>,
}

pub struct PeerMap {
    pub peers: HashMap<x25519::PublicKey, PeerInfo>,
}

pub struct MeshManager {
    // Owns the TUN device + boringtun tunnel
    // Applies peer map updates
    // Handles key rotation
}
```

**Dependencies:** `boringtun = "0.7"`, `x25519-dalek`, `runesh-tun`, `runesh-acl`, `tokio`, `ipnet`

**How to get latest info:**
- boringtun API: `docs.rs/boringtun/0.7.0`
- WireGuard protocol: `wireguard.com/protocol/`
- x25519-dalek: `docs.rs/x25519-dalek`

**Tests:** Integration tests with two MeshManager instances talking through loopback.

### Phase 3: `runesh-relay` (DERP relay server)

A standalone DERP relay that forwards encrypted WireGuard packets between peers that cannot establish direct connections. Runs on the cheap VPS nodes.

**What it does:**
- Listen on TCP/443 (TLS-wrapped)
- Accept connections from mesh clients
- Route packets by WireGuard public key (DERP addressing)
- Forward opaque encrypted packets (never decrypts)
- Health endpoint for DNS failover checks

**DERP frame format (from Tailscale source):**
```
Frame type (1 byte):
  0x01 = ServerKey      (server -> client: server's public key)
  0x02 = ClientInfo      (client -> server: client info JSON)
  0x04 = SendPacket      (client -> server: forward to peer)
  0x05 = RecvPacket      (server -> client: packet from peer)
  0x06 = KeepAlive
  0x07 = NotePreferred   (client -> server: this is my preferred relay)
  0x08 = PeerGone        (server -> client: peer disconnected)
  0x09 = PeerPresent     (server -> client: peer connected)
  0x0a = WatchConns      (client -> server: subscribe to peer events)
  0x0b = ClosePeer       (server -> client: close connection to peer)
  0x0c = Ping
  0x0d = Pong
  0x0e = Health          (server -> client: health status)
  0x0f = Restarting      (server -> client: graceful shutdown notice)
  0x10 = ForwardPacket   (server -> server: mesh relay forwarding)

Length: 4 bytes big-endian
Payload: variable
```

**Reference implementation:** `github.com/tailscale/tailscale/blob/main/derp/derp.go` (~800 lines) and `derp/derp_server.go` (~1500 lines).

**Dependencies:** `tokio`, `rustls`, `runesh-core` (for error types, metrics)

**How to get latest info:**
- DERP protocol: Read `tailscale/derp/derp.go` directly. The frame constants are at the top of the file. Search for `const frameServerKey`.
- Testing: Use `tailscale debug derp` to test against your relay.

**Tests:** Start relay on localhost, connect two clients, send packets between them, verify delivery.

### Phase 4: `runesh-coord` (Tailscale-compatible coordination server)

This is the control plane that official Tailscale clients connect to. Implements the TS2021 protocol.

**What it does:**
- TS2021 Noise IK handshake (client authenticates server, then server authenticates client)
- Node registration: client sends machine key + node key, server stores them
- Peer map distribution: server computes which peers each node should know about (based on ACLs), pushes updates
- DERP map distribution: tells clients which DERP relays exist
- DNS config: MagicDNS records for the tenant's mesh
- Auth: pre-auth keys for unattended enrollment, OIDC for interactive

**TS2021 handshake flow:**
```
1. Client GET /key?v=<capabilities>
   Server responds with server's Noise public key

2. Client POST /ts2021 with Upgrade: websocket
   Connection upgrades to WebSocket

3. Inside WebSocket: Noise IK handshake
   Client knows server's public key (from step 1)
   Client sends: e, es, s, ss (Noise IK initiator)
   Server responds: e, ee, se (Noise IK responder)

4. After handshake: H2C (HTTP/2 cleartext) runs inside the Noise tunnel
   All control messages are HTTP/2 requests within this encrypted channel

5. Control messages (as HTTP/2 inside Noise):
   - MapRequest: client requests its peer map
   - MapResponse: server sends peer map (streaming, kept open for updates)
   - RegisterRequest: client registers its node
   - SetDNS: server pushes DNS config
```

**Reference:** Headscale source code:
- `github.com/juanfont/headscale/tree/main/hscontrol` (start here)
- `protocol_ts2021.go` (Noise handshake)
- `noise.go` (Noise crypto)
- `poll.go` (long-poll for map updates)
- `mapper/mapper.go` (builds MapResponse from DB state)

**Dependencies:** `snow` (Noise protocol in Rust), `h2`, `tokio`, `axum`, `sqlx` (Postgres), `runesh-mesh`, `runesh-acl`, `runesh-core`, `runesh-auth`

**How to get latest info:**
- `snow` crate (Noise protocol): `docs.rs/snow` -- latest stable
- Headscale protocol changes: watch `github.com/juanfont/headscale/releases`
- Tailscale client protocol: `github.com/tailscale/tailscale/tree/main/control/controlclient`
- MapResponse protobuf: `github.com/tailscale/tailscale/blob/main/tailcfg/tailcfg.go` (the Go structs that serialize to JSON, not actual protobuf)

**Key challenge:** The MapResponse format is complex. It contains peer public keys, endpoints, allowed IPs, DERP map, DNS config, user profiles, and more. Headscale's `mapper.go` is the best reference for what fields are required vs optional.

**Tests:** Use `tailscale up --login-server=http://localhost:PORT` to test with a real Tailscale client. Also unit-test the Noise handshake and MapResponse serialization.

### Phase 5: `runesh-proxy` (pingora reverse proxy)

The public-facing reverse proxy running on cheap VPS nodes. Routes incoming HTTPS to backends connected via the WireGuard mesh.

**What it does:**
- SNI-based routing: read hostname from TLS ClientHello, route to correct backend
- HTTP/HTTPS reverse proxy with HTTP/2, gRPC, WebSocket support
- ACME certificate management (DNS-01 via Cloudflare API)
- Multi-tenant: each tenant's resources isolated by hostname/path
- Certificate distribution: obtain on one node, push to others via mesh
- Health endpoint for DNS failover
- Backend health checks (active + passive)
- Access control layers: geo/IP filter, time window, identity gate, authorization

**pingora ProxyHttp implementation:**
```rust
struct HelvetiaProxy {
    config: Arc<ProxyConfig>,  // tenant -> resource -> backend mapping
    mesh: Arc<MeshManager>,    // for reaching backends via mesh
    certs: Arc<CertStore>,     // ACME-managed certificates
}

#[async_trait]
impl ProxyHttp for HelvetiaProxy {
    type CTX = RequestContext; // per-request tenant + resource state

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        // 1. Extract hostname from session (SNI or Host header)
        // 2. Look up tenant + resource from config
        // 3. Return HttpPeer pointing to mesh IP of the backend agent
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<bool> {
        // Access control layers:
        // Layer 1: geo, IP, rate limit
        // Layer 2: time window, maintenance mode
        // Layer 3: identity (SSO cookie, mTLS, PSK)
        // Layer 4: authorization (group, path, method)
    }
}
```

**Dependencies:** `pingora = "0.8"`, `pingora-proxy = "0.8"`, `instant-acme = "0.7"`, `runesh-mesh`, `runesh-core`, `runesh-auth`

**How to get latest info:**
- pingora docs: `docs.rs/pingora/0.8.0` and `github.com/cloudflare/pingora/tree/main/docs`
- pingora examples: `github.com/cloudflare/pingora/tree/main/pingora-proxy/examples`
- Cloudflare API for DNS-01: `developers.cloudflare.com/api/resources/dns/subresources/records/`

**Tests:** Start proxy on localhost, mock a backend, send HTTPS requests, verify routing. Test SNI routing with `curl --resolve`.

### Phase 6: `runesh-agent` (endpoint daemon)

The single binary running on every managed device. Composes all features.

**What it does:**
- Service installation (Windows SCM, systemd, launchd) via `service-manager`
- Enrollment: generate keypair, authenticate via pre-auth key or OIDC, receive signed cert
- Heartbeat loop (configurable, default 60s)
- Join WireGuard mesh on enrollment
- Task queue: controller pushes tasks, agent executes
- Feature modules: inventory, remote, desktop, monitoring (feature-gated)
- Self-update: download new binary, rename-replace, restart service
- QUIC transport primary, WebSocket fallback

**Enrollment flow:**
```
1. Agent first run: generate x25519 keypair (machine key)
2. Agent calls controller: POST /api/v1/machines/register
   Body: { machine_key, os, hostname, version }
   Auth: pre-auth key header or redirect to OIDC
3. Controller validates, creates machine record
4. Controller returns: { node_key, mesh_ip, derp_map, peer_map, cert }
5. Agent configures WireGuard tunnel with returned keys
6. Agent starts heartbeat loop over QUIC/mesh
7. All subsequent comms go through the encrypted mesh
```

**Dependencies:** `quinn = "0.11"`, `rustls = "0.23"`, `rcgen = "0.13"`, `service-manager`, `self_update`, `runesh-mesh`, `runesh-inventory`, `runesh-remote`, `runesh-desktop`, `runesh-core`, `runesh-auth`, `runesh-telemetry`

**How to get latest info:**
- quinn: `docs.rs/quinn/0.11`
- service-manager: `docs.rs/service-manager`
- self_update: `docs.rs/self_update` (for GitHub release download pattern)
- Tailscale enrollment: study `tailscale up --authkey=<key>` flow

**Tests:** Integration test: start controller, enroll agent, verify mesh connectivity.

---

## VPS Deployment Model

```
Each VPS runs:
  1. runesh-proxy    (pingora, port 443/80)
  2. runesh-relay    (DERP, port 443 via SNI routing or separate port)
  3. runesh-agent    (to join the mesh itself)

The controller runs separately (your own infra or a VPS):
  1. runesh-coord    (Tailscale coordination server)
  2. Axum API        (management API + web UI)
  3. Postgres        (state)

VPS nodes are stateless. They get their config (proxy routes, certs, DERP keys)
from the controller via the mesh. If a VPS dies, DNS failover removes it.
New VPS: install agent, enroll, it auto-configures as proxy+relay.
```

---

## Dependency Versions (verified current as of 2026-04-19)

| Crate | Version | Purpose | Docs |
|-------|---------|---------|------|
| `boringtun` | 0.7.0 | WireGuard userspace | docs.rs/boringtun/0.7.0 |
| `pingora` | 0.8.0 | Reverse proxy framework | docs.rs/pingora/0.8.0 |
| `quinn` | 0.11.x | QUIC transport | docs.rs/quinn/0.11 |
| `rustls` | 0.23.x | TLS | docs.rs/rustls/0.23 |
| `rcgen` | 0.13.2 | X.509 cert generation | docs.rs/rcgen/0.13 |
| `instant-acme` | 0.7.1 | ACME client | docs.rs/instant-acme/0.7 |
| `snow` | latest | Noise protocol (for TS2021) | docs.rs/snow |
| `h2` | latest | HTTP/2 (inside Noise tunnel) | docs.rs/h2 |
| `x25519-dalek` | 2.x | Key exchange | docs.rs/x25519-dalek |
| `ipnet` | 2.x | IP network types | docs.rs/ipnet |
| `service-manager` | 0.7.x | Cross-platform service install | docs.rs/service-manager |
| `self_update` | 0.41.x | Binary self-update | docs.rs/self_update |
| `hickory-dns` | 0.25.x | DNS server (MagicDNS) | docs.rs/hickory-dns |
| `sqlx` | 0.8.x | Postgres | docs.rs/sqlx |

---

## Rules for AI Subagents

### Getting latest crate info

**ALWAYS** check the actual docs.rs page or crates.io page for a crate before using it. Do not rely on training data for API signatures. Specifically:

1. Use `context7` MCP tool to fetch latest docs for any crate before writing code
2. Check `Cargo.toml` of existing RUNESH crates for workspace dependency patterns
3. Follow existing code conventions in the RUNESH repo (see `CLAUDE.md`)

### Code conventions (from RUNESH CLAUDE.md)

- Error handling: `thiserror` enums with `status_code()` + `error_code()`
- Async: tokio runtime, `spawn_blocking` for CPU/blocking work
- Logging: `tracing` with structured fields
- Serialization: `serde` with `#[serde(rename_all = "snake_case")]`
- Feature gates: Optional integrations behind Cargo features
- Platform code: `#[cfg(target_os = "...")]` with shared trait abstractions
- Security: Validate all external input at boundaries

### Adding a new crate

1. Create `crates/runesh-{name}/` with `Cargo.toml` + `src/lib.rs`
2. Workspace auto-discovers via `members = ["crates/*"]`
3. Follow the error pattern: `error.rs` with thiserror enum
4. Feature-gate optional deps: `#[cfg(feature = "axum")]`
5. Use workspace deps: `tokio = { workspace = true }`
6. Platform-specific deps under `[target.'cfg(...)'.dependencies]`
7. Update `CLAUDE.md` with crate description

### Do NOT

- Use em dashes or en dashes anywhere
- Add AI attribution to commits
- Modify infrastructure without confirmation
- Guess at API signatures from training data; look them up
- Create massive PRs; keep changes focused (one crate per PR ideally)

### Reference projects to study (in order of relevance)

| Project | URL | What to learn |
|---------|-----|---------------|
| Headscale | github.com/juanfont/headscale | TS2021 protocol, coordination server, MapResponse format |
| Tailscale (client) | github.com/tailscale/tailscale | DERP protocol, control client, tailcfg types |
| EasyTier | github.com/EasyTier/EasyTier | Rust WireGuard mesh, boringtun usage patterns |
| Firezone | github.com/firezone/firezone | Rust connlib, SANS-IO NAT traversal, snownet |
| pingora examples | github.com/cloudflare/pingora/tree/main/pingora-proxy/examples | ProxyHttp trait usage |
