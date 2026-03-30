# RUNESH

Shared code connector for Rust + Next.js + shadcn/ui projects — and the foundation for a Rust-native IT management platform.

Instead of copy-pasting the same auth, networking, remote access, and deployment config across projects, RUNESH provides a single source of truth. Improvements here propagate to all consumer projects.

## Install the CLI

```bash
cargo install --path crates/runesh-cli

# Optional: set RUNESH_PATH so it finds shared code from any directory
export RUNESH_PATH="$HOME/Documents/GitHub/RUNESH"
```

## Create a New Project

```bash
# Create a new repo with RUNESH integration (interactive)
runesh new my-project

# Fully automated with specific crates
runesh new my-project -y --local -c "core,auth,inventory" -d "My cool project"

# With GitHub repo creation
runesh new my-project --github --private --org my-org

# Scaffold inside an existing directory
runesh init my-app
```

The `new` command creates a full Cargo workspace with selected RUNESH crates, CLAUDE.md with dev conventions, .gitignore, git init, and optional GitHub repo via `gh` CLI.

The `init` command scaffolds a full-stack app (Rust/Axum + Next.js + optional Tauri desktop + Chrome extension) with interactive feature selection.

## Crates

### Core Infrastructure

| Crate | What it provides |
|-------|-----------------|
| **runesh-core** | `AppError`, `Pagination`, `RateLimiter`, `BroadcastRegistry` (WebSocket), `save_upload`, DB pool, request ID + logging + CORS + security headers middleware, Prometheus metrics, health checks, graceful shutdown, cross-platform service installer |
| **runesh-auth** | OIDC discovery + PKCE, JWT access/refresh tokens, session management, `AuthStore` trait, Axum middleware + handlers |
| **runesh-cli** | `runesh new` (project repo creator) + `runesh init` (full-stack app scaffolder) |

### Remote Management

| Crate | What it provides |
|-------|-----------------|
| **runesh-inventory** | Cross-platform hardware/software inventory — CPU, RAM, disk, GPU, BIOS, battery, network, installed software, processes. Uses `sysinfo` + WMI (Windows) / `/proc`+`/sys` (Linux) / `system_profiler` (macOS). Feature-gated Axum REST handlers. |
| **runesh-remote** | Remote file explorer + PTY terminal over WebSocket. Path traversal prevention, sandbox security, chunked uploads, audit logging. PTY via `portable-pty` (ConPTY on Windows, Unix PTY on Linux/macOS). |
| **runesh-desktop** | Remote desktop sharing — screen capture (DXGI/CoreGraphics/X11), frame encoding (JPEG/PNG/Zstd), input injection (SendInput/CGEvent/XTest), multi-cursor support with software overlay rendering + X11 MPX, clipboard sync, multi-monitor. |

### Filesystem & Networking

| Crate | What it provides |
|-------|-----------------|
| **runesh-vfs** | Virtual filesystem — files appear natively in OS file explorer (like OneDrive). Windows Cloud Filter API for placeholder files with cloud icons; Linux/macOS FUSE. 4 write modes: ReadOnly, WriteThrough, WriteLocal, WriteOverlay. Copy-on-write overlay for multi-tenant (schools: teacher originals + student personal overlay spaces). LRU cache. |
| **runesh-tun** | Cross-platform TUN device abstraction (Windows wintun + Linux /dev/net/tun) for virtual networking. |

### Desktop & UI

| Crate / Package | What it provides |
|----------------|-----------------|
| **runesh-tauri** | Tauri v2 helpers — TOML config management, system tray, process control, Windows UAC elevation |
| **@runesh/ui** | React/Next.js components — `AppSidebar`, `DashboardShell`, Novel WYSIWYG editor, `DataTable`, `AuthProvider`, `ThemeProvider`, API client with auto token refresh, PKCE utilities, WebSocket hooks, OKLCH theme with dark mode |

### Templates

| File | Purpose |
|------|---------|
| `templates/Dockerfile` | Multi-stage: Node frontend + Rust backend + Caddy proxy |
| `templates/compose.yaml` | PostgreSQL + app with health checks |
| `templates/.env.example` | Environment variables template |
| `templates/tauri/` | Tauri v2 desktop app scaffold |

## Quick Start

### Backend

```toml
# Pick the crates you need
runesh-core = { path = "../RUNESH/crates/runesh-core" }
runesh-auth = { path = "../RUNESH/crates/runesh-auth" }
runesh-inventory = { path = "../RUNESH/crates/runesh-inventory" }
runesh-remote = { path = "../RUNESH/crates/runesh-remote" }
runesh-desktop = { path = "../RUNESH/crates/runesh-desktop" }
runesh-vfs = { path = "../RUNESH/crates/runesh-vfs" }
```

```rust
use runesh_core::{AppError, Pagination, PaginatedResponse, shutdown_signal};
use runesh_core::middleware::{cors, health, logging, request_id};
use runesh_auth::{OidcProvider, AuthStore};
use runesh_inventory::{collect_inventory, CollectorConfig};
use runesh_remote::{RemoteState, handlers as remote_handlers};
use runesh_desktop::{DesktopState, handlers as desktop_handlers};
use runesh_vfs::{VfsConfig, WriteMode, OverlayProvider, MountRegistry};
```

### Frontend

```json
"@runesh/ui": "file:../RUNESH/packages/ui"
```

```tsx
import { AppSidebar } from "@runesh/ui/components/layout/app-sidebar"
import { DashboardShell } from "@runesh/ui/components/layout/dashboard-shell"
import { DataTable } from "@runesh/ui/components/ui/data-table"
import { api } from "@runesh/ui/lib/api-client"
```

## Consumer Projects

| Project | Description | Stack |
|---------|-------------|-------|
| **RUMMZ** | Media management | Rust/Axum + Next.js |
| **HARUMI** | Business suite | Rust/Actix + Next.js |
| **HARUMI-NET** | WireGuard overlay network | Rust/Axum + Next.js + Tauri |
| **HARUMI-DEPLOY** | PXE network boot + OS deployment | Rust/Axum + Next.js |
| **MoodleNG** | Learning management | Rust/Axum + Next.js |

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full IT suite roadmap — 22+ crates covering agent, mesh networking, monitoring, EDR, service desk, backup, patch management, and PXE deployment.

## Documentation

- [docs/USAGE.md](docs/USAGE.md) — Detailed integration guide with examples
- [docs/ROADMAP.md](docs/ROADMAP.md) — Full IT management suite roadmap
- [CLAUDE.md](CLAUDE.md) — Project structure, conventions, and development workflow
