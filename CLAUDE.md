# RUNESH - Shared Code Connector

Shared code repository for Rust + Next.js + shadcn/ui projects.

## Workflow Rules

- **Every change MUST go through a Pull Request (PR).** No direct commits to `main`.
- **Before creating a PR, always merge `main` into the feature branch** to ensure the latest changes are included.
- **Every frontend change MUST be verified using the Playwright MCP** before committing.
- **Always use `bun`.** Never use `npm` or `yarn`.
- **Always test changes end-to-end** (docker rebuild, playwright) before claiming they work. Compiling is not testing.

## Attribution

- **Never add Claude/AI as co-author, contributor, or any form of attribution in commits, PRs, or code.**

## AI Assistant Rules

- **Never do more than what was asked.** Do not add features, configs, gitignore rules, or "improvements" that were not explicitly requested. If in doubt, ask first.
- **Never modify infrastructure (docker, CI, env, volumes, databases) without explicit confirmation.** Changing volume names, postgres versions, or compose structure can destroy data.
- **Never commit screenshots, test artifacts, or temporary files** to the repo. Clean up after yourself but do not modify .gitignore unless asked.
- **Never use em dashes or en dashes** anywhere in commits, release notes, or prose. Rewrite the sentence to avoid them entirely.
- **Keep release bodies short.** No filler prose, no walls of text.

## Writing Pull Requests

- Title starts with a past-tense verb: `Added`, `Fixed`, `Refactored`, etc.
- Body MUST contain `resolve #<issue-number>` to auto-close the linked issue (if applicable)
- One PR per issue. Keep changes focused.
- Include a `## Summary` with bullet points and a `## Test plan`
- Squash merge to main

## Structure

```
RUNESH/
├── packages/
│   └── ui/                              # @mydrift/runesh-ui - Shared React/Next.js components
│       └── src/
│           ├── components/
│           │   ├── editor/              # Novel WYSIWYG editor + custom table, slash commands
│           │   ├── layout/              # AppSidebar, DashboardShell, SearchBar, PageHeader
│           │   ├── providers/           # ThemeProvider, QueryProvider
│           │   └── ui/                  # sidebar.tsx, data-table.tsx (shadcn)
│           ├── fonts/                   # Chiron GoRound TC font configuration
│           ├── hooks/                   # useIsMobile
│           ├── lib/                     # api-client, token-store, auth-pkce, utils (cn)
│           └── styles/                  # globals.css (OKLCH theme, dark mode, editor styles)
├── crates/
│   ├── runesh-acl/                      # Tailscale-compatible HuJSON ACL parser + evaluator + diff
│   ├── runesh-appliance/                # Uniform network appliance driver trait (OPNsense/UniFi/FortiGate/etc.)
│   ├── runesh-asset/                    # Hardware asset tracking: lifecycle, warranty, depreciation
│   ├── runesh-audit/                    # Append-only hash-chained audit log with tamper detection
│   ├── runesh-auth/                     # OIDC + JWT + Axum middleware + AuthStore trait + mesh enrollment
│   ├── runesh-backup/                   # Backup engine: content-addressed chunking, dedup, snapshots, retention
│   ├── runesh-baseline/                 # Declarative baselines with inheritance, drift detection, enforcement
│   ├── runesh-cli/                      # Project scaffolding CLI (init templates, compose, env)
│   ├── runesh-coord/                    # Tailscale-compatible coordination: Noise IK handshake, node registry, map builder
│   ├── runesh-dns/                      # MagicDNS, split DNS, zone management, service discovery
│   ├── runesh-flow/                     # Network flow collector: NetFlow/sFlow/IPFIX, top-N aggregations
│   ├── runesh-mdm/                      # Mobile device management: enrollment, config profiles, remote actions
│   ├── runesh-core/                     # AppError, RateLimiter, WS broadcast, file upload, DB pool
│   ├── runesh-desktop/                  # Remote desktop: screen capture, encoding, input injection (Win/Mac/Linux)
│   ├── runesh-mesh/                     # WireGuard mesh: key management, peer maps, IP allocation, tunnel orchestration
│   ├── runesh-monitor/                  # Check engine (HTTP/TCP/ping/disk/command), alert state machine
│   ├── runesh-notify/                   # Notification dispatch: webhook, Slack, Discord, Ntfy, email (SMTP)
│   ├── runesh-patch/                    # Patch management: ring-based rollout, CVE correlation, maintenance windows
│   ├── runesh-pkg/                      # Cross-platform package manager trait: apt, dnf, pacman, winget, brew
│   ├── runesh-proxy/                    # Reverse proxy: resource config, routing, access control, load balancing
│   ├── runesh-relay/                    # DERP relay: forwards encrypted WireGuard packets between peers over TCP
│   ├── runesh-inventory/                # Hardware/software inventory collection (cross-platform)
│   ├── runesh-ipam/                     # IP address management: prefixes, VLANs, allocation, utilization
│   ├── runesh-jobs/                     # Typed job/task model with idempotency, retry, and queue management
│   ├── runesh-license/                  # Software license tracking: entitlements, utilization, renewal alerts
│   ├── runesh-remote/                   # Remote file explorer + CLI over WebSocket with PTY
│   ├── runesh-tauri/                    # Tauri helpers (tray, process mgmt, config, elevation)
│   ├── runesh-telemetry/                # Sentry/GlitchTip error reporting + tracing layer
│   ├── runesh-stun/                     # NAT traversal: STUN, hole punching, connection strategy
│   ├── runesh-vault/                    # Encrypted key-value secret store with rotation and JIT decryption
│   ├── runesh-docker/                   # Docker/Podman workload driver via bollard
│   ├── runesh-hyperv/                   # Hyper-V workload driver via PowerShell (Windows)
│   ├── runesh-proxmox/                  # Proxmox VE workload driver via REST API
│   ├── runesh-vmware/                   # VMware vCenter/ESXi workload driver via REST API
│   ├── runesh-winget/                   # WinGet REST source server
│   ├── runesh-workload/                 # Workload driver trait (docker/proxmox/vmware/hyperv)
│   ├── runesh-tun/                      # Cross-platform TUN device (Windows wintun + Linux)
│   └── runesh-vfs/                      # Virtual filesystem with cloud provider + overlay writes
├── templates/                           # Dockerfile + compose.yaml for new projects
├── docs/
│   └── USAGE.md                         # Detailed integration guide with examples
├── Cargo.toml                           # Rust workspace
├── package.json                         # bun workspace root
```

## Source Projects

| Component | Source | Why |
|-----------|--------|-----|
| UI Frame (sidebar + toolbar) | RUMMZ | Best composable shadcn sidebar with search command palette |
| Font config | HARUMI-NET | Best practice: multi-weight loading, display=swap, system fallbacks |
| Novel editor + custom table | HARUMI | Only project with full Novel/Tiptap editor + custom extensions |
| API client + token store | HARUMI | Best: auto token refresh serialization, 401 retry |
| Data table | HARUMI-NET | Generic sortable/paginated with server-side support |
| OIDC auth | HARUMI + HARUMI-NET | HARUMI's features on HARUMI-NET's Axum middleware |
| AppError | HARUMI + RUMMZ | Identical pattern across all projects, generalized |
| Rate limiter | RUMMZ | Sliding window per IP with Axum middleware |
| WebSocket broadcast | HARUMI + RUMMZ | Per-room pub/sub with tokio broadcast channels |
| File upload | HARUMI + MoodleNG | Multipart handler + XHR progress tracking |
| TUN device | HARUMI-NET | Cross-platform virtual network interface |
| CLI scaffolding | All projects | Project init with templates, compose, env files |
| Tauri helpers | HARUMI-NET | System tray, process management, config, Windows elevation |
| Docker template | RUMMZ | Multi-stage: Node + Rust + Caddy |
| Theme/CSS | RUMMZ | OKLCH color system with dark mode |

## Consumer Projects

- RUMMZ - Media management (Rust/Axum + Next.js)
- HARUMI - Business suite (Rust/Actix + Next.js)
- HARUMI-NET - WireGuard overlay network (Rust/Axum + Next.js + Tauri)
- MoodleNG - Learning management (Rust/Axum + Next.js)

## Quick Reference

See [docs/USAGE.md](docs/USAGE.md) for integration guide. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full IT suite roadmap (~23 crates).

### Frontend
```bash
bun add @mydrift/runesh-ui
```

### Backend
```toml
runesh-core = { path = "../RUNESH/crates/runesh-core" }
runesh-auth = { path = "../RUNESH/crates/runesh-auth" }
runesh-inventory = { path = "../RUNESH/crates/runesh-inventory" }
runesh-remote = { path = "../RUNESH/crates/runesh-remote" }
runesh-desktop = { path = "../RUNESH/crates/runesh-desktop" }
runesh-vfs = { path = "../RUNESH/crates/runesh-vfs" }
```

## Remote Management Crates

### runesh-inventory
Cross-platform hardware/software inventory: CPU, RAM, disk, GPU, network, BIOS, battery, installed software, processes. Uses `sysinfo` + WMI (Windows) / `/proc`+`/sys` (Linux) / `system_profiler` (macOS). Feature-gated Axum REST handlers.

### runesh-remote
Remote file explorer and CLI over WebSocket. File operations with path traversal prevention and sandbox security. PTY-based terminal sessions via `portable-pty` (ConPTY on Windows, Unix PTY on Linux/macOS). Chunked uploads, audit logging, configurable policies.

### runesh-vfs
Cross-platform virtual filesystem that shows remote files natively in the OS file explorer (like OneDrive). Windows uses Cloud Filter API (cfapi) for placeholder files with cloud icons; Linux/macOS use FUSE. Supports 4 write modes: ReadOnly, WriteThrough, WriteLocal, WriteOverlay. The overlay mode enables copy-on-write for multi-tenant scenarios (schools: teachers maintain originals, students get personal overlay spaces where only their edits consume storage). LRU cache with configurable eviction.

### runesh-telemetry
Sentry/GlitchTip error reporting wired into the existing `tracing` stack. Drop-in init in `main.rs` that does nothing unless `RUNESH_SENTRY_DSN` is set, so it's safe to leave in every binary. Provides a `tracing-subscriber` layer that forwards `WARN`/`ERROR` events as Sentry events automatically, plus an optional Axum/Tower middleware (feature `axum`) that captures request context. Because GlitchTip is wire-compatible with the Sentry SDK protocol, the same crate works against either backend. Just point the DSN at your self-hosted GlitchTip instance.

Env vars: `RUNESH_SENTRY_DSN`, `RUNESH_ENV`, `RUNESH_SAMPLE_RATE`, `RUNESH_TELEMETRY_DEBUG`.

### runesh-desktop
Remote desktop sharing with screen capture (DXGI on Windows, CoreGraphics on macOS, X11/XShm on Linux), frame encoding (JPEG/PNG/Zstd), input injection (SendInput/CGEvent/XTest), clipboard sync, multi-cursor support, and multi-monitor support. Wayland architecture via xdg-desktop-portal ready.

---

## Development Workflow

### Prerequisites

- **Rust**: `rustup` with stable toolchain (edition 2024)
- **Bun**: For frontend packages (`bun install`)
- **Platform SDKs**: Windows SDK (for `windows` crate), Xcode CLT (macOS), `libfuse3-dev` (Linux)

### Build Commands

```bash
# Check everything compiles
cargo check

# Check a specific crate
cargo check -p runesh-core

# Build all crates (debug)
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Generate docs
cargo doc --no-deps --open

# Frontend
cd packages/ui && bun install && bun run build
```

### Git Branch Naming

Use `type/short-description` format:

| Prefix | Usage | Example |
|--------|-------|---------|
| `feat/` | New feature | `feat/hardware-inventory` |
| `fix/` | Bug fix | `fix/path-traversal-check` |
| `refactor/` | Code restructuring | `refactor/error-handling` |
| `chore/` | Dependencies, CI, tooling | `chore/update-deps` |
| `docs/` | Documentation only | `docs/usage-guide` |
| `test/` | Adding/fixing tests | `test/overlay-provider` |

- Branch from `main`
- Keep names lowercase, use hyphens (not underscores)
- Keep names short but descriptive

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): short description

Optional longer body explaining why, not what.
```

**Types**: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `perf`, `ci`
**Scope**: crate name without `runesh-` prefix, e.g. `core`, `auth`, `desktop`, `remote`, `inventory`, `vfs`, `ui`

Examples:
```
feat(desktop): add multi-cursor support with overlay rendering
fix(remote): prevent path traversal via symlink resolution
chore(deps): update sysinfo to 0.35
refactor(core): extract rate limiter into generic backend trait
docs(vfs): add overlay provider usage examples
```

### Adding a New Crate

1. Create `crates/runesh-{name}/` with `Cargo.toml` + `src/lib.rs`
2. The workspace auto-discovers via `members = ["crates/*"]`
3. Follow existing patterns:
   - `error.rs` with `thiserror` enum + `status_code()` + `error_code()`
   - Feature-gated `axum` integration: `#[cfg(feature = "axum")]`
   - Use workspace deps: `tokio = { workspace = true }`
   - Platform-specific deps under `[target.'cfg(...)'.dependencies]`
4. Add crate description to this CLAUDE.md under the appropriate section
5. Add integration example to `docs/USAGE.md`

### Code Conventions

- **Error handling**: `thiserror` enums, `?` operator, map internal errors to user-safe messages
- **Async**: tokio runtime, `spawn_blocking` for CPU/blocking work
- **Logging**: `tracing` crate with structured fields (`tracing::info!(key = %value, "message")`)
- **Serialization**: `serde` with `#[serde(rename_all = "snake_case")]`
- **Feature gates**: Optional integrations behind Cargo features (axum, redis, sqlx, etc.)
- **Platform code**: `#[cfg(target_os = "...")]` with shared trait abstractions
- **Security**: Validate all external input at boundaries, sanitize paths, block path traversal
