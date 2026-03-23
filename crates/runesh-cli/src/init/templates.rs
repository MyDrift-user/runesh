use super::ProjectConfig;

pub fn cargo_workspace(c: &ProjectConfig) -> String {
    let mut content = format!(r#"[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
axum = {{ version = "0.8", features = ["ws", "multipart"] }}
tower = "0.5"
tower-http = {{ version = "0.6", features = ["cors", "trace", "compression-gzip"] }}
tokio = {{ version = "1", features = ["full"] }}
sqlx = {{ version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter"] }}
thiserror = "2"
uuid = {{ version = "1", features = ["v4", "serde"] }}
chrono = {{ version = "0.4", features = ["serde"] }}
dotenvy = "0.15"

# RUNESH shared crates
{core_dep}
"#, core_dep = c.cargo_dep("runesh-core"));

    if c.with_auth {
        content.push_str(&c.cargo_dep("runesh-auth"));
        content.push('\n');
    }

    content
}

pub fn server_cargo(c: &ProjectConfig) -> String {
    let mut deps = format!(r#"[package]
name = "{name}-server"
version.workspace = true
edition.workspace = true

[dependencies]
axum.workspace = true
tower.workspace = true
tower-http.workspace = true
tokio.workspace = true
sqlx.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
uuid.workspace = true
chrono.workspace = true
dotenvy.workspace = true
runesh-core.workspace = true
"#, name = c.name);

    if c.with_auth {
        deps.push_str("runesh-auth.workspace = true\n");
    }

    deps
}

pub fn server_main(c: &ProjectConfig) -> String {
    let mut imports = String::from(r#"use std::net::SocketAddr;

use axum::{routing::get, Router, Json, middleware};
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

use runesh_core::shutdown_signal;
"#);

    if c.with_rate_limit {
        imports.push_str("use runesh_core::rate_limit::{RateLimiter, rate_limit_layer};\n");
    }
    if c.with_auth {
        imports.push_str("use runesh_auth::axum_middleware::{auth_middleware, JwtSecret, AuthExemptPaths};\n");
    }

    let mut setup = String::from(r#"
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let pool = runesh_core::db::create_pool(None)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let app = Router::new()
        .route("/api/v1/health", get(health))
"#);

    if c.with_rate_limit {
        setup.push_str(&format!(r#"        .layer(middleware::from_fn(move |req, next| {{
            let limiter = RateLimiter::new(100, 60);
            rate_limit_layer(limiter, true, req, next)
        }}))
"#));
    }

    if c.with_auth {
        setup.push_str(r#"        .layer(middleware::from_fn(auth_middleware))
        .layer(axum::Extension(JwtSecret(
            std::env::var("JWT_SECRET").expect("JWT_SECRET must be set"),
        )))
        .layer(axum::Extension(AuthExemptPaths::default()))
"#);
    }

    setup.push_str(&format!(r#"        .layer(runesh_core::middleware::cors::cors_layer(&["*"]))
        .layer(middleware::from_fn(runesh_core::middleware::logging::logging_middleware))
        .layer(middleware::from_fn(runesh_core::middleware::request_id::request_id_middleware))
        .with_state(pool);

    let port: u16 = std::env::var("PORT").unwrap_or_else(|_| "{port}".into()).parse().unwrap();
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Listening on {{addr}}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}}

async fn health(axum::extract::State(pool): axum::extract::State<PgPool>) -> Json<serde_json::Value> {{
    runesh_core::middleware::health::health_handler(axum::extract::State(pool)).await
}}
"#, port = c.port));

    format!("{imports}{setup}")
}

pub fn initial_migration(c: &ProjectConfig) -> String {
    let mut sql = String::from(r#"CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    avatar_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#);

    if c.with_auth {
        sql.push_str(r#"
ALTER TABLE users ADD COLUMN oidc_sub VARCHAR(255) UNIQUE;
ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN last_login_at TIMESTAMPTZ;

CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash VARCHAR(255) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#);
    }

    sql
}

pub fn web_package_json(c: &ProjectConfig) -> String {
    format!(r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "dev": "next dev",
    "build": "next build",
    "start": "next start",
    "lint": "eslint"
  }},
  "dependencies": {{
    {ui_dep},
    "@base-ui/react": "^1.2.0",
    "@tanstack/react-query": "^5.90.21",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "lucide-react": "^0.577.0",
    "next": "16.1.6",
    "next-themes": "^0.4.6",
    "react": "19.2.3",
    "react-dom": "19.2.3",
    "shadcn": "^4.0.5",
    "sonner": "^2.0.7",
    "tailwind-merge": "^3.5.0",
    "tw-animate-css": "^1.4.0"
  }},
  "devDependencies": {{
    "@tailwindcss/postcss": "^4",
    "@types/node": "^20",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "eslint": "^9",
    "eslint-config-next": "16.1.6",
    "tailwindcss": "^4",
    "typescript": "^5"
  }}
}}"#, name = c.name, ui_dep = c.npm_ui_dep())
}

pub const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["dom", "dom.iterable", "esnext"],
    "allowJs": true,
    "skipLibCheck": true,
    "strict": true,
    "noEmit": true,
    "esModuleInterop": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "preserve",
    "incremental": true,
    "plugins": [{ "name": "next" }],
    "paths": { "@/*": ["./src/*"] }
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
"#;

pub const NEXT_CONFIG: &str = r#"import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
};

export default nextConfig;
"#;

pub const POSTCSS_CONFIG: &str = r#"const config = {
  plugins: {
    "@tailwindcss/postcss": {},
  },
};
export default config;
"#;

pub const GLOBALS_CSS_IMPORT: &str = r#"@import "@runesh/ui/styles/globals.css";
"#;

pub fn layout_tsx(c: &ProjectConfig, is_desktop: bool) -> String {
    let title = if is_desktop { format!("{} Desktop", c.name) } else { c.name.clone() };
    let tauri_import = if is_desktop {
        r#"import { TitleBar } from "@runesh/ui/components/layout/title-bar";
import { useTauri } from "@runesh/ui/hooks/use-tauri";
"#
    } else { "" };
    let title_bar = if is_desktop {
        format!(r#"        {{/* Frameless window title bar */}}
        <TitleBar title="{}" />"#, title)
    } else { String::new() };

    format!(r#""use client";

import "./globals.css";
import {{ Toaster }} from "sonner";
import {{ ThemeProvider }} from "@runesh/ui/components/providers/theme-provider";
import {{ QueryProvider }} from "@runesh/ui/components/providers/query-provider";
import {{ AuthProvider }} from "@runesh/ui/components/providers/auth-provider";
import {{ CHIRON_GOROUND_URL, FONT_FAMILY_SANS }} from "@runesh/ui/fonts";
{tauri_import}

export default function RootLayout({{ children }}: {{ children: React.ReactNode }}) {{
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <title>{title}</title>
        <link rel="stylesheet" href={{CHIRON_GOROUND_URL}} />
      </head>
      <body style={{{{ fontFamily: FONT_FAMILY_SANS }}}} className="antialiased">
        <ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
          <QueryProvider>
            <AuthProvider>
{title_bar}
              {{children}}
            </AuthProvider>
          </QueryProvider>
        </ThemeProvider>
        <Toaster />
      </body>
    </html>
  );
}}
"#, title = title, tauri_import = tauri_import, title_bar = title_bar)
}

pub fn home_page(c: &ProjectConfig) -> String {
    format!(r#"import {{ PageHeader }} from "@runesh/ui/components/layout/page-header";

export default function Home() {{
  return (
    <div className="p-6">
      <PageHeader title="{name}" description="Welcome to your new project." />
    </div>
  );
}}
"#, name = c.name)
}

pub const UTILS_TS: &str = r#"export { cn } from "@runesh/ui/lib/utils";
"#;

pub fn dot_env(c: &ProjectConfig) -> String {
    format!(r#"DATABASE_URL=postgres://{db}:{db}@localhost:5432/{db}
JWT_SECRET=change-this-to-a-random-64-char-string-in-production!!
PORT={port}
RUST_LOG=info
"#, db = c.db_name, port = c.port)
}

pub const GITIGNORE: &str = r#"# Rust
target/

# Node
node_modules/
.next/
out/

# Bun
bun.lock

# Environment
.env
.env.local
.env.production

# IDE
.vscode/
.idea/
*.swp
*.swo

# OS
.DS_Store
Thumbs.db
Desktop.ini

# Tauri
src-tauri/target/
src-tauri/gen/

# Database
*.db
*.db-shm
*.db-wal

# Build artifacts
dist/
*.exe
*.msi
*.dmg
*.AppImage
*.deb

# Database
*.db
*.db-shm
*.db-wal
"#;

pub fn dockerfile(c: &ProjectConfig) -> String {
    format!(r#"# ── Stage 1: Build Next.js frontend ─────────────────────────────────────────
FROM oven/bun:latest AS web-builder
WORKDIR /build
COPY web/package.json web/bun.lock* ./
RUN bun install --frozen-lockfile
COPY web/ .
RUN bun run build

# ── Stage 2: Build Rust backend ────────────────────────────────────────────
FROM rust:1-bookworm AS rust-builder
WORKDIR /build
RUN cargo install cargo-chef
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin {name}-server

# ── Stage 3: Runtime ───────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=amd64" \
    -o /usr/local/bin/caddy && chmod +x /usr/local/bin/caddy

WORKDIR /app
COPY --from=web-builder /build/.next/standalone ./web/
COPY --from=web-builder /build/.next/static ./web/.next/static/
COPY --from=web-builder /build/public ./web/public/
COPY --from=rust-builder /build/target/release/{name}-server ./backend
COPY migrations/ ./migrations/

RUN printf ':8080 {{\n  handle /api/* {{\n    reverse_proxy localhost:{port}\n  }}\n  handle /ws {{\n    reverse_proxy localhost:{port}\n  }}\n  handle {{\n    reverse_proxy localhost:3000\n  }}\n}}\n' > /etc/Caddyfile

RUN printf '#!/bin/sh\nset -e\ncaddy start --config /etc/Caddyfile &\ncd /app/web && PORT=3000 node server.js &\ncd /app && ./backend &\nwait -n\n' > /app/start.sh && chmod +x /app/start.sh

EXPOSE 8080
CMD ["/app/start.sh"]
"#, name = c.name, port = c.port)
}

pub fn compose_yaml(c: &ProjectConfig) -> String {
    format!(r#"services:
  db:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_DB: {db}
      POSTGRES_USER: {db}
      POSTGRES_PASSWORD: ${{POSTGRES_PASSWORD:-changeme}}
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U {db}"]
      interval: 5s
      timeout: 5s
      retries: 5
    networks:
      - internal

  app:
    build: .
    restart: unless-stopped
    ports:
      - "${{APP_PORT:-8080}}:8080"
    environment:
      DATABASE_URL: postgres://{db}:${{POSTGRES_PASSWORD:-changeme}}@db:5432/{db}
      JWT_SECRET: ${{JWT_SECRET}}
      PORT: "{port}"
    depends_on:
      db:
        condition: service_healthy
    networks:
      - internal

volumes:
  pgdata:

networks:
  internal:
    driver: bridge
"#, db = c.db_name, port = c.port)
}

pub fn tauri_cargo(c: &ProjectConfig) -> String {
    format!(r#"[package]
name = "{name}-desktop"
version = "0.1.0"
edition = "2024"

[lib]
name = "{snake}_desktop"
crate-type = ["lib", "cdylib", "staticlib"]

[build-dependencies]
tauri-build = {{ version = "2", features = [] }}

[dependencies]
tauri = {{ version = "2", features = ["tray-icon", "image-png"] }}
tauri-plugin-shell = "2"
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
{tauri_dep}
"#, name = c.name, snake = c.snake_name, tauri_dep = c.cargo_dep("runesh-tauri"))
}

pub fn tauri_main(c: &ProjectConfig) -> String {
    format!(r#"#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {{
    {snake}_desktop::run();
}}
"#, snake = c.snake_name)
}

pub fn tauri_lib(c: &ProjectConfig) -> String {
    format!(r#"use std::sync::Mutex;
use serde::{{Deserialize, Serialize}};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {{
    pub server: String,
}}

pub struct AppState {{
    pub config: Mutex<AppConfig>,
}}

#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> AppConfig {{
    state.config.lock().unwrap().clone()
}}

#[tauri::command]
fn save_config(state: tauri::State<'_, AppState>, server: String) -> Result<String, String> {{
    let mut config = state.config.lock().unwrap();
    config.server = server;
    runesh_tauri::config::save_config("{name}", &*config)?;
    Ok("Saved".into())
}}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {{
    let config: AppConfig = runesh_tauri::config::load_or_create("{name}");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {{ config: Mutex::new(config) }})
        .invoke_handler(tauri::generate_handler![get_config, save_config])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}}
"#, name = c.name)
}

pub fn tauri_conf(c: &ProjectConfig) -> String {
    format!(r#"{{
  "productName": "{name}",
  "version": "0.1.0",
  "identifier": "com.runesh.{name}",
  "build": {{
    "frontendDist": "../web/out",
    "devUrl": "http://localhost:3000",
    "beforeDevCommand": "cd ../web && bun dev",
    "beforeBuildCommand": "cd ../web && bun run build"
  }},
  "app": {{
    "windows": [
      {{
        "title": "{name}",
        "width": 1200,
        "height": 800,
        "center": true,
        "decorations": false,
        "resizable": true,
        "minWidth": 800,
        "minHeight": 600
      }}
    ],
    "security": {{
      "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; connect-src 'self' https: wss:; img-src 'self' data: https:"
    }}
  }},
  "bundle": {{
    "active": true,
    "targets": "all",
    "icon": [
      "icons/icon.ico",
      "icons/icon.png"
    ]
  }}
}}"#, name = c.name)
}

pub const TAURI_CAPABILITIES: &str = r#"{
  "identifier": "default",
  "description": "Default capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open"
  ]
}
"#;

pub fn claude_md(c: &ProjectConfig) -> String {
    let mut features = Vec::new();
    if c.with_auth { features.push("OIDC auth (runesh-auth)"); }
    if c.with_rate_limit { features.push("Rate limiting"); }
    if c.with_ws { features.push("WebSocket broadcast"); }
    if c.with_upload { features.push("File upload"); }
    if c.with_docker { features.push("Docker deployment"); }
    if c.with_tauri { features.push("Tauri v2 desktop"); }

    format!(r#"# {name}

## Stack
- Backend: Rust (Axum) + PostgreSQL (SQLx)
- Frontend: Next.js + React + shadcn/ui + Tailwind CSS v4
- Shared code: RUNESH ({source})
- Package manager: bun

## Features
{features}

## Structure
```
{name}/
├── crates/{name}-server/    # Axum API server
├── migrations/              # SQLx PostgreSQL migrations
├── web/                     # Next.js frontend
{tauri}├── Cargo.toml               # Rust workspace
└── .env                     # Environment variables
```

## Development
```
# Backend
cargo run -p {name}-server

# Frontend
cd web && bun dev
{tauri_dev}```
"#,
        name = c.name,
        source = match &c.source {
            super::RuneshSource::Git(url) => url.as_str(),
            super::RuneshSource::Local(path) => path.as_str(),
        },
        features = features.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n"),
        tauri = if c.separate_desktop {
            format!("├── desktop/                  # Desktop Next.js frontend\n├── crates/{}-desktop/       # Desktop Rust backend\n├── src-tauri/                # Tauri v2 shell\n", c.name)
        } else if c.with_tauri {
            "├── src-tauri/                # Tauri v2 desktop app\n".into()
        } else { String::new() },
        tauri_dev = if c.with_tauri { "\n# Desktop\ncd src-tauri && cargo tauri dev".into() } else { String::new() },
    )
}

// ── Separate desktop templates ──────────────────────────────────────────────

pub fn desktop_package_json(c: &ProjectConfig) -> String {
    format!(r#"{{
  "name": "{name}-desktop",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "dev": "next dev -p 3100",
    "build": "next build",
    "start": "next start"
  }},
  "dependencies": {{
    {ui_dep},
    "@tauri-apps/api": "^2.5.0",
    "@tauri-apps/plugin-shell": "^2.3.5",
    "@base-ui/react": "^1.2.0",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "lucide-react": "^0.577.0",
    "next": "16.1.6",
    "next-themes": "^0.4.6",
    "react": "19.2.3",
    "react-dom": "19.2.3",
    "shadcn": "^4.0.5",
    "sonner": "^2.0.7",
    "tailwind-merge": "^3.5.0",
    "tw-animate-css": "^1.4.0"
  }},
  "devDependencies": {{
    "@tailwindcss/postcss": "^4",
    "@types/node": "^20",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "tailwindcss": "^4",
    "typescript": "^5"
  }}
}}"#, name = c.name, ui_dep = c.npm_ui_dep())
}

pub const NEXT_CONFIG_STATIC: &str = r#"import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
  trailingSlash: true,
};

export default nextConfig;
"#;

pub fn desktop_home_page(c: &ProjectConfig) -> String {
    format!(r#"import {{ PageHeader }} from "@runesh/ui/components/layout/page-header";

export default function Home() {{
  return (
    <div className="p-6">
      <PageHeader title="{name} Desktop" description="Desktop application." />
    </div>
  );
}}
"#, name = c.name)
}

pub fn desktop_backend_cargo(c: &ProjectConfig) -> String {
    format!(r#"[package]
name = "{name}-desktop"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
runesh-core.workspace = true
"#, name = c.name)
}

pub fn desktop_backend_lib(c: &ProjectConfig) -> String {
    format!(r#"//! Desktop-specific backend logic for {name}.
//!
//! This crate contains Tauri commands and business logic that is
//! specific to the desktop app and not shared with the web server.

use serde::{{Deserialize, Serialize}};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopStatus {{
    pub version: String,
    pub connected: bool,
}}

pub fn get_status() -> DesktopStatus {{
    DesktopStatus {{
        version: env!("CARGO_PKG_VERSION").to_string(),
        connected: false,
    }}
}}
"#, name = c.name)
}

pub fn tauri_cargo_separate(c: &ProjectConfig) -> String {
    format!(r#"[package]
name = "{name}-tauri"
version = "0.1.0"
edition = "2024"

[lib]
name = "{snake}_tauri"
crate-type = ["lib", "cdylib", "staticlib"]

[build-dependencies]
tauri-build = {{ version = "2", features = [] }}

[dependencies]
tauri = {{ version = "2", features = ["tray-icon", "image-png"] }}
tauri-plugin-shell = "2"
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
{name}-desktop = {{ path = "../crates/{name}-desktop" }}
{tauri_dep}
"#, name = c.name, snake = c.snake_name, tauri_dep = c.cargo_dep("runesh-tauri"))
}

pub fn tauri_conf_separate(c: &ProjectConfig) -> String {
    format!(r#"{{
  "productName": "{name}",
  "version": "0.1.0",
  "identifier": "com.runesh.{name}",
  "build": {{
    "frontendDist": "../desktop/out",
    "devUrl": "http://localhost:3100",
    "beforeDevCommand": "cd ../desktop && bun dev",
    "beforeBuildCommand": "cd ../desktop && bun run build"
  }},
  "app": {{
    "windows": [
      {{
        "title": "{name}",
        "width": 1200,
        "height": 800,
        "center": true,
        "decorations": false,
        "resizable": true,
        "minWidth": 800,
        "minHeight": 600
      }}
    ],
    "security": {{
      "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; connect-src 'self' https: wss:; img-src 'self' data: https:"
    }}
  }},
  "bundle": {{
    "active": true,
    "targets": "all",
    "icon": [
      "icons/icon.ico",
      "icons/icon.png"
    ]
  }}
}}"#, name = c.name)
}
