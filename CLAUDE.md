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
│   ├── runesh-core/                     # AppError, RateLimiter, WS broadcast, file upload, DB pool
│   └── runesh-tun/                      # Cross-platform TUN device (Windows wintun + Linux)
├── templates/                           # Dockerfile + compose.yaml for new projects
├── docs/
│   └── USAGE.md                         # Detailed integration guide with examples
├── Cargo.toml                           # Rust workspace
├── package.json                         # pnpm workspace root
└── pnpm-workspace.yaml
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
```
