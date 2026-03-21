# RUNESH

Shared code connector for Rust + Next.js + shadcn/ui projects.

Instead of copy-pasting the same sidebar, editor, auth flow, error handling, and deployment config across projects, RUNESH provides a single source of truth. Improvements here propagate to all consumer projects.

## What's included

### Frontend (`@runesh/ui`)

| Category | Components |
|----------|-----------|
| **Layout** | `AppSidebar`, `DashboardShell`, `SearchBar`, `PageHeader` |
| **Editor** | Novel WYSIWYG with `TableMenu`, `BubbleMenu`, `SlashCommand`, collapsible headings, search highlighting |
| **Data** | `DataTable` (sortable, paginated, server-side support), `ConfirmDialog` |
| **Providers** | `AuthProvider`, `ThemeProvider`, `QueryProvider` |
| **Auth** | `api` client (auto token refresh, 401 retry, file upload with progress), `token-store`, PKCE utilities |
| **Hooks** | `useIsMobile`, `useWebSocket` (auto-reconnect, auth), `usePermissions` |
| **Utils** | `cn()`, `formatFileSize()`, `formatRelativeTime()`, `formatDateLabel()`, pagination types |
| **Styles** | `globals.css` (OKLCH theme, dark mode), font config (Chiron GoRound TC) |

### Backend Rust Crates

| Crate | What it provides |
|-------|-----------------|
| **runesh-core** | `AppError` (7 variants with HTTP mapping), `Pagination` extractor + `PaginatedResponse`, `RateLimiter`, `BroadcastRegistry` (WebSocket), `save_upload`, `create_pool`, `shutdown_signal`, request ID + logging + CORS middleware, health check handler, cross-platform service installer |
| **runesh-auth** | OIDC discovery + PKCE, JWT access/refresh tokens, Axum middleware, `AuthStore` trait |
| **runesh-tun** | Cross-platform TUN device (Windows wintun + Linux /dev/net/tun) |

### Templates

| File | Purpose |
|------|---------|
| `templates/Dockerfile` | Multi-stage: Node frontend + Rust backend + Caddy proxy |
| `templates/compose.yaml` | PostgreSQL + app with health checks |
| `templates/.env.example` | All environment variables for a new project |

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

## Consumer projects

- **RUMMZ** - Media management (Axum + Next.js)
- **HARUMI** - Business suite (Actix + Next.js)
- **HARUMI-NET** - WireGuard overlay network (Axum + Next.js + Tauri)
- **MoodleNG** - Learning management (Axum + Next.js)

## Documentation

See [docs/USAGE.md](docs/USAGE.md) for detailed integration guide with examples.
