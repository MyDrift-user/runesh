# RUNESH

Shared code connector for Rust + Next.js + shadcn/ui projects.

Instead of copy-pasting the same sidebar, editor, auth flow, error handling, and deployment config across projects, RUNESH provides a single source of truth. Improvements here propagate to all consumer projects.

## What's included

### Frontend (`@runesh/ui`)

| Category | Components |
|----------|-----------|
| **Layout** | `AppSidebar`, `DashboardShell`, `SearchBar`, `PageHeader` |
| **Editor** | Novel WYSIWYG with custom `TableMenu`, `BubbleMenu`, `SlashCommand`, collapsible headings, search highlighting |
| **Data** | `DataTable` (sortable, paginated, server-side support) |
| **Providers** | `ThemeProvider`, `QueryProvider` |
| **Auth** | `api` client (auto token refresh, 401 retry), `token-store`, PKCE utilities |
| **Styles** | `globals.css` (OKLCH theme, dark mode), font config (Chiron GoRound TC) |
| **Hooks** | `useIsMobile` |
| **Utils** | `cn()` (clsx + tailwind-merge) |

### Backend Rust Crates

| Crate | What it provides |
|-------|-----------------|
| **runesh-core** | `AppError` (HTTP status mapping), `RateLimiter` (sliding window), `BroadcastRegistry` (WebSocket pub/sub), `save_upload` (multipart handler), `create_pool` (SQLx PostgreSQL) |
| **runesh-auth** | OIDC discovery + PKCE, JWT access/refresh tokens, Axum middleware, `AuthStore` trait for project-specific extensibility |
| **runesh-tun** | Cross-platform TUN device (Windows wintun + Linux /dev/net/tun) |

### Templates

| File | Purpose |
|------|---------|
| `templates/Dockerfile` | Multi-stage build: Node frontend + Rust backend + Caddy proxy |
| `templates/compose.yaml` | PostgreSQL + app service with health checks |

## Quick start

### Frontend

```json
// In your project's package.json
"dependencies": {
  "@runesh/ui": "file:../RUNESH/packages/ui"
}
```

```tsx
import { AppSidebar } from "@runesh/ui/components/layout/app-sidebar"
import { DashboardShell } from "@runesh/ui/components/layout/dashboard-shell"
import { api } from "@runesh/ui/lib/api-client"
import { DataTable } from "@runesh/ui/components/ui/data-table"
```

### Backend

```toml
# In your project's Cargo.toml
runesh-core = { path = "../RUNESH/crates/runesh-core" }
runesh-auth = { path = "../RUNESH/crates/runesh-auth" }
```

```rust
use runesh_core::{AppError, RateLimiter};
use runesh_auth::{OidcProvider, AuthStore};
```

## Consumer projects

- **RUMMZ** - Media management (Axum + Next.js)
- **HARUMI** - Business suite (Actix + Next.js)
- **HARUMI-NET** - WireGuard overlay network (Axum + Next.js + Tauri)
- **MoodleNG** - Learning management (Axum + Next.js)

## Documentation

See [docs/USAGE.md](docs/USAGE.md) for detailed integration guide with examples.
