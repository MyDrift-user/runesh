# RUNESH Usage Guide

Complete integration guide for all shared components.

---

## Table of Contents

- [Frontend (@runesh/ui)](#frontend-runeshui)
  - [Installation](#installation)
  - [Layout: AppSidebar + DashboardShell](#layout-appsidebar--dashboardshell)
  - [Layout: SearchBar](#layout-searchbar)
  - [Layout: PageHeader](#layout-pageheader)
  - [Editor: Novel WYSIWYG](#editor-novel-wysiwyg)
  - [Data Table](#data-table)
  - [API Client](#api-client)
  - [Token Store](#token-store)
  - [Auth PKCE Utilities](#auth-pkce-utilities)
  - [Providers](#providers)
  - [Fonts](#fonts)
  - [Styles](#styles)
- [Backend Crates](#backend-crates)
  - [runesh-core: AppError](#runesh-core-apperror)
  - [runesh-core: RateLimiter](#runesh-core-ratelimiter)
  - [runesh-core: WebSocket Broadcast](#runesh-core-websocket-broadcast)
  - [runesh-core: File Upload](#runesh-core-file-upload)
  - [runesh-core: Database Pool](#runesh-core-database-pool)
  - [runesh-auth: OIDC Authentication](#runesh-auth-oidc-authentication)
  - [runesh-auth: Axum Middleware](#runesh-auth-axum-middleware)
  - [runesh-auth: AuthStore Trait](#runesh-auth-authstore-trait)
  - [runesh-tun: TUN Device](#runesh-tun-tun-device)
- [Templates](#templates)

---

## Frontend (@runesh/ui)

### Installation

Add to your project's `package.json`:

```json
{
  "dependencies": {
    "@runesh/ui": "file:../RUNESH/packages/ui"
  }
}
```

The package expects these peer dependencies (which your Next.js project already has):
`next`, `react`, `react-dom`

### Layout: AppSidebar + DashboardShell

The main UI frame used across all projects. `DashboardShell` provides the full layout (sidebar + sticky toolbar + content area). `AppSidebar` is the navigation sidebar with configurable items.

```tsx
// app/layout.tsx
import { AppSidebar } from "@runesh/ui/components/layout/app-sidebar"
import { DashboardShell } from "@runesh/ui/components/layout/dashboard-shell"
import { Home, Settings, Users } from "lucide-react"

const navItems = [
  { title: "Dashboard", href: "/", icon: Home },
  { title: "Users", href: "/users", icon: Users },
  { title: "Settings", href: "/settings", icon: Settings, adminOnly: true },
]

export default function Layout({ children }) {
  const { user, logout } = useAuth()

  const sidebar = (
    <AppSidebar
      navItems={navItems}
      user={user}
      onLogout={logout}
      brandIcon={<MyLogo />}
      brandName="My App"
    />
  )

  return (
    <DashboardShell
      sidebar={sidebar}
      isAuthenticated={!!user}
      shortcutHint={<kbd className="...">Ctrl K</kbd>}
    >
      {children}
    </DashboardShell>
  )
}
```

**AppSidebar props:**
- `navItems` - Array of `{ title, href, icon, adminOnly? }`
- `user` - `{ username, email?, role? }` or null
- `onLogout` - Callback when sign out is clicked
- `brandIcon` - React node for the logo
- `brandName` - Text displayed next to logo
- `groupLabel` - Label above nav items (default: "Navigation")

**DashboardShell props:**
- `sidebar` - The sidebar component to render
- `searchBar` - Optional command palette component
- `isLoading` / `isAuthenticated` - Auth state
- `shortcutHint` / `toolbarExtra` - Toolbar content
- `contentClassName` - CSS class for main area (default: "p-4 md:p-6")

### Layout: SearchBar

Command palette (Ctrl+K) with pluggable search.

```tsx
import { SearchBar } from "@runesh/ui/components/layout/search-bar"

async function onSearch(query: string) {
  const res = await api.get(`/search?q=${query}`)
  return res.items.map(item => ({
    id: item.id,
    title: item.name,
    href: `/items/${item.id}`,
    group: "Results",
  }))
}

<DashboardShell
  sidebar={sidebar}
  searchBar={<SearchBar onSearch={onSearch} placeholder="Search items..." />}
>
```

### Layout: PageHeader

Standard page title + description + action buttons.

```tsx
import { PageHeader } from "@runesh/ui/components/layout/page-header"
import { Button } from "@/components/ui/button"

<PageHeader title="Users" description="Manage user accounts">
  <Button>Add User</Button>
</PageHeader>
```

### Editor: Novel WYSIWYG

Full-featured rich text editor with slash commands, tables, and more.

```tsx
import { EditorRoot, EditorContent } from "novel"
import { defaultExtensions } from "@runesh/ui/editor/extensions"
import { slashCommand, suggestionItems } from "@runesh/ui/editor/slash-command"
import { EditorBubbleMenu } from "@runesh/ui/editor/bubble-menu"
import { TableMenu } from "@runesh/ui/editor/table-menu"
import { SearchHighlightExtension } from "@runesh/ui/editor/search-highlight-extension"
import { CollapsibleHeadingExtension } from "@runesh/ui/editor/collapsible-heading-extension"

const extensions = [...defaultExtensions, slashCommand, SearchHighlightExtension, CollapsibleHeadingExtension]

<EditorRoot>
  <EditorContent
    extensions={extensions}
    initialContent={content}
    onUpdate={({ editor }) => onChange(editor.getJSON())}
    editorProps={{
      attributes: {
        class: "prose dark:prose-invert max-w-3xl mx-auto px-8 pb-32",
      },
    }}
  >
    <EditorBubbleMenu />
  </EditorContent>
</EditorRoot>
```

**Included extensions:** StarterKit, links, task lists, code blocks (lowlight), tables (resizable), underline, highlight, color, drag handle, trailing node, drag-handle ghost table fix.

**Custom extensions:**
- `TableMenu` - Floating toolbar above active table (add/remove rows/cols, toggle headers, delete)
- `CollapsibleHeadingExtension` - Toggle carets on headings to collapse/expand sections
- `SearchHighlightExtension` - Highlight search terms in the document

### Data Table

Generic sortable, searchable, paginated table.

```tsx
import { DataTable, type DataTableColumn } from "@runesh/ui/components/ui/data-table"

interface User { id: string; name: string; email: string; role: string }

const columns: DataTableColumn<User>[] = [
  { key: "name", header: "Name", getValue: (u) => u.name },
  { key: "email", header: "Email", getValue: (u) => u.email },
  {
    key: "role",
    header: "Role",
    getValue: (u) => u.role,
    renderCell: (u) => <Badge>{u.role}</Badge>,
  },
]

<DataTable
  columns={columns}
  data={users}
  loading={isLoading}
  renderRowActions={(user) => <DropdownMenu>...</DropdownMenu>}
  searchPlaceholder="Search users..."
/>
```

**Server-side pagination:**

```tsx
<DataTable
  columns={columns}
  data={page.items}
  serverPagination={{
    total: page.total,
    page: currentPage,
    pageSize: 25,
    onPageChange: setCurrentPage,
    onPageSizeChange: setPageSize,
  }}
  hideSearch  // search handled server-side
/>
```

### API Client

Fetch wrapper with auto token refresh, 401 retry, and file upload with progress.

```tsx
import { api, uploadFile } from "@runesh/ui/lib/api-client"
import { setTokenPrefix } from "@runesh/ui/lib/token-store"

// Set once at app startup to avoid key collisions
setTokenPrefix("myapp")

// Standard CRUD
const users = await api.get<User[]>("/api/users")
const user = await api.post<User>("/api/users", { name: "Alice" })
await api.delete("/api/users/123")

// File upload with progress
await uploadFile("/api/uploads", formData, (pct) => {
  console.log(`${pct}% uploaded`)
})
```

### Token Store

LocalStorage-based token persistence. Configure the prefix to avoid collisions:

```tsx
import { setTokenPrefix, storeTokens, clearTokens, getAccessToken } from "@runesh/ui/lib/token-store"

setTokenPrefix("rummz")  // Keys: rummz_access_token, rummz_refresh_token, etc.

// After login
storeTokens(accessToken, refreshToken, expiresIn, { id, name, email, role, avatar_url })

// On logout
clearTokens()
```

### Auth PKCE Utilities

Frontend OIDC flow with PKCE (used with runesh-auth backend).

```tsx
import {
  generateCodeVerifier, generateCodeChallenge, generateState,
  buildAuthUrl, storePending, retrievePending,
} from "@runesh/ui/lib/auth-pkce"

// Start login
const verifier = generateCodeVerifier()
const challenge = await generateCodeChallenge(verifier)
const state = generateState()
storePending({ verifier, state })

const url = buildAuthUrl({
  authorizationEndpoint: oidcConfig.authorization_endpoint,
  clientId: oidcConfig.client_id,
  redirectUri: window.location.origin + "/auth/callback",
  scope: oidcConfig.scope,
}, challenge, state)

window.location.href = url

// In callback page
const pending = retrievePending()
const res = await api.post("/api/auth/callback", {
  code: searchParams.get("code"),
  code_verifier: pending.verifier,
  redirect_uri: window.location.origin + "/auth/callback",
})
storeTokens(res.access_token, res.refresh_token, res.expires_in, res.user)
```

### Providers

```tsx
// app/layout.tsx
import { ThemeProvider } from "@runesh/ui/components/providers/theme-provider"
import { QueryProvider } from "@runesh/ui/components/providers/query-provider"

<ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
  <QueryProvider staleTime={60000} retry={1}>
    {children}
  </QueryProvider>
</ThemeProvider>
```

### Fonts

Two approaches depending on your setup:

**Next.js SSR (recommended):**
```tsx
import { Chiron_GoRound_TC, Geist_Mono } from "next/font/google"

const chiron = Chiron_GoRound_TC({ variable: "--font-chiron-goround", subsets: ["latin"], display: "swap" })
const mono = Geist_Mono({ variable: "--font-geist-mono", subsets: ["latin"] })

<html className={`${chiron.variable} ${mono.variable}`}>
```

**Static export / Tauri:**
```tsx
import { CHIRON_GOROUND_URL, FONT_FAMILY_SANS } from "@runesh/ui/fonts"

<head>
  <link rel="stylesheet" href={CHIRON_GOROUND_URL} />
</head>
<body style={{ fontFamily: FONT_FAMILY_SANS }}>
```

### Styles

Copy or import `globals.css` for the shared OKLCH theme:

```css
@import "@runesh/ui/styles/globals.css";
```

Or copy the file to your project and customize colors.

---

## Backend Crates

### runesh-core: AppError

Standard error type with HTTP status mapping. Works with Axum's `IntoResponse` and auto-converts SQLx errors.

```rust
use runesh_core::AppError;

async fn get_user(Path(id): Path<Uuid>) -> Result<Json<User>, AppError> {
    let user = sqlx::query_as("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await?;  // RowNotFound -> 404, other -> 500
    Ok(Json(user))
}

// Manual errors
return Err(AppError::BadRequest("Invalid email".into()));
return Err(AppError::Forbidden("Admin only".into()));
```

### runesh-core: RateLimiter

In-memory sliding window rate limiter per IP.

```rust
use axum::middleware;
use runesh_core::rate_limit::{RateLimiter, rate_limit_layer};

let api_limiter = RateLimiter::new(100, 60);   // 100 req/min
let auth_limiter = RateLimiter::new(10, 60);    // 10 req/min

let app = Router::new()
    .nest("/api/v1", api_routes.layer(
        middleware::from_fn(move |req, next| {
            rate_limit_layer(api_limiter.clone(), req, next)
        })
    ))
    .nest("/api/auth", auth_routes.layer(
        middleware::from_fn(move |req, next| {
            rate_limit_layer(auth_limiter.clone(), req, next)
        })
    ));

// Periodic cleanup (call from a background task)
tokio::spawn(async move {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;
        limiter.cleanup();
    }
});
```

### runesh-core: WebSocket Broadcast

Per-room pub/sub using tokio broadcast channels.

```rust
use runesh_core::ws_broadcast::{BroadcastRegistry, ws_broadcast_loop};

let broadcast = BroadcastRegistry::new(128);

// In a handler: send an event to a room
broadcast.send("notifications", serde_json::to_string(&event)?).await;

// WebSocket handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        ws_broadcast_loop(socket, &state.broadcast, "notifications", |msg| {
            tracing::debug!("Client sent: {msg}");
        }).await;
    })
}
```

### runesh-core: File Upload

Multipart file upload to disk with UUID-based filenames.

```rust
use runesh_core::upload::save_upload;

async fn upload(mut multipart: Multipart) -> Result<Json<Value>, AppError> {
    while let Some(field) = multipart.next_field().await? {
        let uploaded = save_upload(field, Path::new("./storage"), 100 * 1024 * 1024).await?;
        // uploaded.filename, uploaded.storage_key, uploaded.size, uploaded.content_type
    }
    Ok(Json(json!({"ok": true})))
}
```

### runesh-core: Database Pool

```rust
use runesh_core::db::create_pool;

let pool = create_pool(None).await?;  // reads DATABASE_URL from env
sqlx::migrate!("./migrations").run(&pool).await?;
```

### runesh-auth: OIDC Authentication

Full OIDC flow with PKCE, token exchange, userinfo, and JWT issuance.

```rust
use runesh_auth::{OidcProvider, OidcSessionStore, TokenConfig};
use runesh_auth::token::issue_access_token;

// At startup
let provider = OidcProvider::from_env().await?.expect("OIDC not configured");
let sessions = OidcSessionStore::new();
let token_config = TokenConfig::new(std::env::var("JWT_SECRET")?);

// Start OIDC flow
let (session_id, auth_url) = sessions.start(&provider, None).await;

// Handle callback
let session = sessions.get_by_state(&state_param).await.unwrap();
let (token_resp, user_info) = provider.exchange_code(&code, &session.code_verifier, None).await?;
sessions.remove(&session.id).await;

// user_info contains: sub, email, name, picture, groups, idp_access_token, idp_refresh_token
```

### runesh-auth: Axum Middleware

```rust
use axum::middleware;
use runesh_auth::axum_middleware::{auth_middleware, JwtSecret, AuthExemptPaths};

let app = Router::new()
    .nest("/api/v1", protected_routes)
    .layer(middleware::from_fn(auth_middleware))
    .layer(Extension(JwtSecret("my-secret".into())))
    .layer(Extension(AuthExemptPaths(vec![
        "/auth/".into(),
        "/health".into(),
        "/ws/".into(),
    ])));

// In handlers, extract claims:
async fn handler(Extension(claims): Extension<Claims>) -> impl IntoResponse {
    format!("Hello, {}", claims.name)
}
```

### runesh-auth: AuthStore Trait

Implement this trait to connect auth to your project's database. This is how you add project-specific logic like MS Graph photo fetching or group-to-role mapping.

```rust
use runesh_auth::store::{AuthStore, AuthUser};
use runesh_auth::oidc::OidcUserInfo;

struct MyStore { pool: PgPool }

#[async_trait]
impl AuthStore for MyStore {
    async fn upsert_user(&self, info: &OidcUserInfo) -> Result<AuthUser, AuthError> {
        // Upsert user in DB
        let user = sqlx::query_as("INSERT INTO users ...")
            .fetch_one(&self.pool).await?;

        // Project-specific: fetch MS Graph photo using info.idp_access_token
        // Project-specific: map info.groups to local roles

        Ok(AuthUser {
            id: user.id.to_string(),
            email: user.email,
            name: user.name,
            role: "user".into(),
            avatar_url: None,
            permissions: vec![],
        })
    }

    async fn get_user_by_id(&self, id: &str) -> Result<AuthUser, AuthError> { ... }
    async fn store_refresh_token(&self, user_id: &str, hash: &str, expires: DateTime<Utc>) -> Result<(), AuthError> { ... }
    async fn consume_refresh_token(&self, hash: &str) -> Result<String, AuthError> { ... }
    async fn revoke_all_refresh_tokens(&self, user_id: &str) -> Result<(), AuthError> { ... }

    // Optional: enable password login
    async fn verify_password(&self, email: &str, password: &str) -> Result<Option<AuthUser>, AuthError> {
        // bcrypt verify...
    }
}
```

### runesh-tun: TUN Device

Cross-platform virtual network interface for overlay networking.

```rust
use runesh_tun::{TunDevice, TunConfig};

let tun = TunDevice::create(TunConfig {
    name: "mynet0".into(),
    address: "100.64.0.1".parse()?,
    netmask: "255.192.0.0".parse()?,
    mtu: 1420,
})?;

// Read/write packets
if let Some(packet) = tun.read_blocking() {
    process_packet(&packet);
}
tun.write(&outgoing_packet)?;
```

Requires Administrator/root privileges. On Windows, needs `wintun.dll` next to the executable.

---

## Templates

### Dockerfile

Copy `templates/Dockerfile` to your project root. Change `YOUR_BINARY` to your actual binary name.

Key features:
- cargo-chef for dependency layer caching
- Next.js standalone output (minimal image size)
- Caddy reverse proxy (frontend on :3000, backend on :3001, exposed on :8080)

### compose.yaml

Copy `templates/compose.yaml` to your project root. Configure via `.env` file:

```env
POSTGRES_DB=myapp
POSTGRES_USER=myapp
POSTGRES_PASSWORD=secretpassword
JWT_SECRET=my-jwt-secret
APP_PORT=8080
```

---

## Project Structure

```
RUNESH/
├── packages/
│   └── ui/                              # @runesh/ui
│       └── src/
│           ├── components/
│           │   ├── editor/              # Novel WYSIWYG editor
│           │   ├── layout/              # AppSidebar, DashboardShell, SearchBar, PageHeader
│           │   ├── providers/           # ThemeProvider, QueryProvider
│           │   └── ui/                  # sidebar.tsx, data-table.tsx
│           ├── fonts/                   # Chiron GoRound TC config
│           ├── hooks/                   # useIsMobile
│           ├── lib/                     # api-client, token-store, auth-pkce, utils
│           └── styles/                  # globals.css
├── crates/
│   ├── runesh-auth/                     # OIDC + JWT + Axum middleware
│   ├── runesh-core/                     # AppError, RateLimiter, WS broadcast, upload, db
│   └── runesh-tun/                      # Cross-platform TUN device
├── templates/                           # Dockerfile + compose.yaml
├── Cargo.toml                           # Rust workspace
├── package.json                         # pnpm workspace
└── pnpm-workspace.yaml
```
