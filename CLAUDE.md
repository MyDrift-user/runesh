# RUNESH - Shared Code Connector

Shared code repository for Rust + Next.js + shadcn/ui projects.

## Structure

```
RUNESH/
├── packages/
│   └── ui/                              # @runesh/ui - Shared React/Next.js components
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
│   ├── runesh-auth/                     # OIDC + JWT + Axum middleware + AuthStore trait
│   ├── runesh-cli/                      # Project scaffolding CLI (init templates, compose, env)
│   ├── runesh-core/                     # AppError, RateLimiter, WS broadcast, file upload, DB pool
│   ├── runesh-desktop/                  # Remote desktop: screen capture, encoding, input injection (Win/Mac/Linux)
│   ├── runesh-inventory/                # Hardware/software inventory collection (cross-platform)
│   ├── runesh-remote/                   # Remote file explorer + CLI over WebSocket with PTY
│   ├── runesh-tauri/                    # Tauri helpers (tray, process mgmt, config, elevation)
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

See [docs/USAGE.md](docs/USAGE.md) for full integration guide.

### Frontend
```json
"@runesh/ui": "file:../RUNESH/packages/ui"
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
**Scope**: crate name without `runesh-` prefix — e.g., `core`, `auth`, `desktop`, `remote`, `inventory`, `vfs`, `ui`

Examples:
```
feat(desktop): add multi-cursor support with overlay rendering
fix(remote): prevent path traversal via symlink resolution
chore(deps): update sysinfo to 0.35
refactor(core): extract rate limiter into generic backend trait
docs(vfs): add overlay provider usage examples
```

### Pull Requests

- One feature/fix per PR — keep PRs focused
- PR title matches the primary commit message format
- Include a `## Summary` with bullet points and a `## Test plan`
- Link related issues if applicable
- Squash merge to main

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
