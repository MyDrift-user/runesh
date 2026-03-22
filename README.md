# RUNESH

Shared code connector for Rust + Next.js + shadcn/ui projects.

Instead of copy-pasting the same sidebar, editor, auth flow, error handling, and deployment config across projects, RUNESH provides a single source of truth. Improvements here propagate to all consumer projects.

## Create a new project

```bash
# Build the CLI once
cargo build -p runesh-cli --release

# Scaffold a new project
./target/release/runesh init my-app
```

The interactive wizard will ask for:
- **Project type**: Web only or Web + Tauri desktop
- **Features**: OIDC auth, rate limiting, WebSocket, file upload, Docker
- **Database name** and **backend port**

It generates the full project structure, links `@runesh/ui`, runs `bun install`, and initializes shadcn/ui.

## What's included

### Frontend (`@runesh/ui`)

| Category | Components |
|----------|-----------|
| **Layout** | `AppSidebar`, `DashboardShell`, `SearchBar`, `PageHeader`, `TitleBar` (Tauri frameless) |
| **Editor** | Novel WYSIWYG with `TableMenu`, `BubbleMenu`, `SlashCommand`, collapsible headings, search highlighting |
| **Data** | `DataTable` (sortable, paginated, server-side support), `ConfirmDialog` |
| **Providers** | `AuthProvider`, `ThemeProvider`, `QueryProvider` |
| **Auth** | `api` client (auto token refresh, 401 retry, file upload with progress), `token-store`, PKCE utilities |
| **Hooks** | `useIsMobile`, `useWebSocket` (auto-reconnect, auth), `usePermissions`, `useWindowControls` (Tauri), `useTauri` (detection) |
| **Utils** | `cn()`, `formatFileSize()`, `formatRelativeTime()`, `formatDateLabel()`, pagination types, `createInvoke()` (typed Tauri IPC) |
| **Styles** | `globals.css` (OKLCH theme, dark mode), font config (Chiron GoRound TC) |

### Backend Rust Crates

| Crate | What it provides |
|-------|-----------------|
| **runesh-core** | `AppError` (7 variants with HTTP mapping), `Pagination` extractor + `PaginatedResponse`, `RateLimiter`, `BroadcastRegistry` (WebSocket), `save_upload`, `create_pool`, `shutdown_signal`, request ID + logging + CORS middleware, health check handler, cross-platform service installer |
| **runesh-auth** | OIDC discovery + PKCE, JWT access/refresh tokens, Axum middleware, `AuthStore` trait |
| **runesh-tauri** | Tauri v2 utilities: TOML config management, system tray setup, process finder/launcher, Windows UAC elevation |
| **runesh-tun** | Cross-platform TUN device (Windows wintun + Linux /dev/net/tun) |

### Templates

| File | Purpose |
|------|---------|
| `templates/Dockerfile` | Multi-stage: Node frontend + Rust backend + Caddy proxy |
| `templates/compose.yaml` | PostgreSQL + app with health checks |
| `templates/.env.example` | All environment variables for a new project |
| `templates/tauri/` | Tauri v2 desktop app scaffold (Cargo.toml, lib.rs, tauri.conf.json, capabilities) |

## Quick start

### Frontend

```json
"@runesh/ui": "file:../RUNESH/packages/ui"
```

```tsx
import { AppSidebar } from "@runesh/ui/components/layout/app-sidebar"
import { DashboardShell } from "@runesh/ui/components/layout/dashboard-shell"
import { AuthProvider, useAuth } from "@runesh/ui/components/providers/auth-provider"
import { api } from "@runesh/ui/lib/api-client"
import { DataTable } from "@runesh/ui/components/ui/data-table"
import { useWebSocket } from "@runesh/ui/hooks/use-websocket"
```

### Backend

```toml
runesh-core = { path = "../RUNESH/crates/runesh-core" }
runesh-auth = { path = "../RUNESH/crates/runesh-auth" }
```

```rust
use runesh_core::{AppError, Pagination, PaginatedResponse, shutdown_signal};
use runesh_core::middleware::{cors, health, logging, request_id};
use runesh_auth::{OidcProvider, AuthStore};
```

### Tauri Desktop

```toml
# In your Tauri app's Cargo.toml
runesh-tauri = { path = "../RUNESH/crates/runesh-tauri" }
```

```tsx
// Frontend: frameless window with custom title bar
import { TitleBar } from "@runesh/ui/components/layout/title-bar"
import { useTauri } from "@runesh/ui/hooks/use-tauri"
import { createInvoke } from "@runesh/ui/lib/tauri-invoke"
```

## Consumer projects

- **RUMMZ** - Media management (Axum + Next.js)
- **HARUMI** - Business suite (Actix + Next.js)
- **HARUMI-NET** - WireGuard overlay network (Axum + Next.js + Tauri)
- **MoodleNG** - Learning management (Axum + Next.js)

## Documentation

See [docs/USAGE.md](docs/USAGE.md) for detailed integration guide with examples.
