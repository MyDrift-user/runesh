//! `runesh new` — Create a new project repo wired to RUNESH.
//!
//! Produces a single-container, single-port skeleton where the Rust
//! server serves the JSON API at `/api/*` and falls back to a static
//! Next.js SPA. No reverse proxy, no cross-service DNS, no CORS.
//!
//! - Cargo workspace pulling runesh-* crates via `git + tag` pinned to
//!   the CLI's own version, never path refs.
//! - `crates/{snake}-server` with Axum + sqlx-postgres + config-via-env +
//!   `tower-http::ServeDir` SPA fallback.
//! - `packages/web` Next.js 15 app with `output: "export"`, Tailwind
//!   v4, TanStack Query, `axios` on a `/api` same-origin base URL.
//! - Multi-stage `Dockerfile`: bun web-builder → rust-builder → slim
//!   runtime (tini, UID 1000, buildkit cache mounts).
//! - `compose.yaml`: just Postgres + server. No `web` service.
//! - `migrations/0001_init.sql` seeded with useful Postgres extensions.
//! - `.env.example`, `.dockerignore`, sensible `.gitignore`.
//! - Root `package.json` as a bun workspace; `CLAUDE.md` + `README.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use console::style;
use dialoguer::{Confirm, Input, MultiSelect};

/// RUNESH crates that are useful to wire into a server's workspace
/// deps via the interactive menu. Not exhaustive; the caller can add
/// more to Cargo.toml by hand, and `--crates a,b,c` accepts names
/// that are not in this list (they still get a `git + tag` entry).
const AVAILABLE_CRATES: &[(&str, &str)] = &[
    ("core", "AppError, rate limiter, WS broadcast, file upload"),
    ("auth", "OIDC + JWT + RBAC + Axum middleware"),
    ("audit", "Append-only hash-chained audit log"),
    ("vault", "ChaCha20-Poly1305 encrypted KV"),
    ("notify", "Webhook / Slack / Discord / Ntfy / email"),
    ("monitor", "HTTP/TCP/ping/disk/command checks + alerts"),
    ("jobs", "Typed job queue with retry + idempotency"),
    ("baseline", "Declarative baselines + drift + remediation"),
    ("patch", "Ring-based patch rollout with soak windows"),
    ("inventory", "Cross-platform HW + SW inventory (agent-side)"),
    ("remote", "Remote file explorer + PTY over WebSocket"),
    ("desktop", "Remote desktop capture + input injection"),
    ("vfs", "Cloud-Filter / FUSE virtual filesystem"),
    ("telemetry", "Sentry / GlitchTip error reporting"),
];

/// Git tag used for all runesh-* deps in the scaffolded Cargo.toml.
/// We pin to the CLI's own published version so the scaffold is
/// reproducible — users who install runesh v0.18.0 get a project that
/// depends on runesh v0.18.0.
fn runesh_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    name: String,
    description: Option<String>,
    crates_arg: Option<String>,
    create_github: bool,
    private: bool,
    org: Option<String>,
    use_local: bool,
    accept_defaults: bool,
) -> Result<(), String> {
    println!(
        "\n  {}  {}\n",
        style("RUNESH").bold().cyan(),
        style("New Project").dim()
    );

    crate::validate::check_project_name(&name)?;
    let snake_name = name.replace('-', "_");
    let root = PathBuf::from(&name);
    if root.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    // ── Description ────────────────────────────────────────────────────

    let description = if let Some(desc) = description {
        desc
    } else if accept_defaults {
        format!("{name} built on RUNESH")
    } else {
        Input::new()
            .with_prompt("Project description")
            .default(format!("{name} built on RUNESH"))
            .interact_text()
            .map_err(|e| e.to_string())?
    };

    // ── Crate selection ────────────────────────────────────────────────

    let selected_crates: BTreeSet<String> = if let Some(crates_str) = crates_arg {
        crates_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if accept_defaults {
        ["core", "auth"].iter().map(|s| s.to_string()).collect()
    } else {
        println!(
            "  {} Select RUNESH crates (space to toggle, enter to confirm):\n",
            style("1/2").dim()
        );
        let labels: Vec<String> = AVAILABLE_CRATES
            .iter()
            .map(|(n, d)| format!("runesh-{n:<10} {d}"))
            .collect();
        let defaults: Vec<bool> = AVAILABLE_CRATES
            .iter()
            .map(|(n, _)| matches!(*n, "core" | "auth"))
            .collect();
        let picked = MultiSelect::new()
            .items(&labels)
            .defaults(&defaults)
            .interact()
            .map_err(|e| e.to_string())?;
        picked
            .into_iter()
            .map(|i| AVAILABLE_CRATES[i].0.to_string())
            .collect()
    };

    // ── GitHub repo ────────────────────────────────────────────────────

    let should_create_github = if create_github {
        true
    } else if accept_defaults {
        false
    } else {
        Confirm::new()
            .with_prompt("Create GitHub repo?")
            .default(false)
            .interact()
            .map_err(|e| e.to_string())?
    };

    // `--local` no longer writes path deps. If set, drop a gitignored
    // .cargo/config.toml that patches the git deps to a sibling RUNESH
    // checkout. The Cargo.toml itself is always git + tag so the
    // project stays portable.
    let local_overlay = use_local;

    println!(
        "\n  {} Creating project '{}' pinned to runesh {}\n",
        style("->").green(),
        style(&name).cyan(),
        style(runesh_tag()).dim()
    );

    // ── Scaffold ───────────────────────────────────────────────────────

    let server_dir = root.join("crates").join(format!("{snake_name}-server"));
    fs::create_dir_all(server_dir.join("src")).map_err(|e| format!("mkdir: {e}"))?;
    fs::create_dir_all(root.join("migrations")).map_err(|e| format!("mkdir: {e}"))?;
    fs::create_dir_all(root.join("docs")).map_err(|e| format!("mkdir: {e}"))?;
    let web_src = root.join("packages").join("web").join("src");
    fs::create_dir_all(web_src.join("app")).map_err(|e| format!("mkdir: {e}"))?;
    fs::create_dir_all(web_src.join("components")).map_err(|e| format!("mkdir: {e}"))?;
    fs::create_dir_all(web_src.join("lib")).map_err(|e| format!("mkdir: {e}"))?;

    fs::write(
        root.join("Cargo.toml"),
        generate_workspace_cargo(&description, &selected_crates),
    )
    .map_err(|e| format!("write Cargo.toml: {e}"))?;

    fs::write(
        server_dir.join("Cargo.toml"),
        generate_server_cargo(&snake_name, &selected_crates),
    )
    .map_err(|e| format!("write server Cargo.toml: {e}"))?;
    fs::write(
        server_dir.join("src").join("main.rs"),
        generate_server_main(&snake_name, &selected_crates),
    )
    .map_err(|e| format!("write main.rs: {e}"))?;

    fs::write(root.join(".gitignore"), GITIGNORE).map_err(|e| format!("write .gitignore: {e}"))?;
    fs::write(root.join(".dockerignore"), DOCKERIGNORE)
        .map_err(|e| format!("write .dockerignore: {e}"))?;
    fs::write(root.join(".env.example"), ENV_EXAMPLE)
        .map_err(|e| format!("write .env.example: {e}"))?;
    fs::write(root.join("Dockerfile"), generate_dockerfile(&snake_name))
        .map_err(|e| format!("write Dockerfile: {e}"))?;
    fs::write(
        root.join("compose.yaml"),
        generate_compose(&name, &snake_name),
    )
    .map_err(|e| format!("write compose.yaml: {e}"))?;
    fs::write(
        root.join("migrations").join("0001_init.sql"),
        INITIAL_MIGRATION,
    )
    .map_err(|e| format!("write migration: {e}"))?;

    // ── Web (Next.js static SPA) ───────────────────────────────────────
    let web_dir = root.join("packages").join("web");
    fs::write(root.join("package.json"), generate_root_package_json(&name))
        .map_err(|e| format!("write root package.json: {e}"))?;
    fs::write(
        web_dir.join("package.json"),
        generate_web_package_json(&name),
    )
    .map_err(|e| format!("write web package.json: {e}"))?;
    fs::write(web_dir.join("next.config.mjs"), WEB_NEXT_CONFIG)
        .map_err(|e| format!("write next.config.mjs: {e}"))?;
    fs::write(web_dir.join("postcss.config.mjs"), WEB_POSTCSS_CONFIG)
        .map_err(|e| format!("write postcss.config.mjs: {e}"))?;
    fs::write(web_dir.join("tsconfig.json"), WEB_TSCONFIG)
        .map_err(|e| format!("write tsconfig.json: {e}"))?;
    fs::write(web_src.join("app").join("globals.css"), WEB_GLOBALS_CSS)
        .map_err(|e| format!("write globals.css: {e}"))?;
    fs::write(
        web_src.join("app").join("layout.tsx"),
        generate_web_layout(&name),
    )
    .map_err(|e| format!("write layout.tsx: {e}"))?;
    fs::write(
        web_src.join("app").join("page.tsx"),
        generate_web_page(&name),
    )
    .map_err(|e| format!("write page.tsx: {e}"))?;
    fs::write(
        web_src.join("components").join("providers.tsx"),
        WEB_PROVIDERS,
    )
    .map_err(|e| format!("write providers.tsx: {e}"))?;
    fs::write(
        web_src.join("components").join("app-shell.tsx"),
        generate_web_app_shell(&name),
    )
    .map_err(|e| format!("write app-shell.tsx: {e}"))?;
    fs::write(
        web_src.join("components").join("page-header.tsx"),
        WEB_PAGE_HEADER,
    )
    .map_err(|e| format!("write page-header.tsx: {e}"))?;
    fs::write(web_src.join("lib").join("api.ts"), WEB_API_CLIENT)
        .map_err(|e| format!("write api.ts: {e}"))?;

    let claude_md = generate_claude_md(&name, &snake_name, &description, &selected_crates);
    fs::write(root.join("CLAUDE.md"), claude_md).map_err(|e| format!("write CLAUDE.md: {e}"))?;

    let md_name = crate::validate::markdown_escape(&name);
    let md_description = crate::validate::markdown_escape(&description);
    fs::write(
        root.join("README.md"),
        format!(
            "# {md_name}\n\n{md_description}\n\nBuilt on [RUNESH](https://github.com/MyDrift-user/runesh) {tag}.\n\nQuick start:\n\n```\ncp .env.example .env\ndocker compose up -d --build\ncurl http://localhost:8080/healthz\n```\n",
            tag = runesh_tag()
        ),
    )
    .map_err(|e| format!("write README.md: {e}"))?;

    if local_overlay {
        let cargo_dir = root.join(".cargo");
        fs::create_dir_all(&cargo_dir).map_err(|e| format!("mkdir .cargo: {e}"))?;
        fs::write(
            cargo_dir.join("config.toml"),
            generate_local_overlay(&selected_crates),
        )
        .map_err(|e| format!("write .cargo/config.toml: {e}"))?;
        println!(
            "  {} Wrote .cargo/config.toml overlay pointing at ../RUNESH; it is gitignored.",
            style("->").green()
        );
    }

    // ── Git init ───────────────────────────────────────────────────────

    println!("  {} Initializing git repository...", style("->").green());
    run_cmd("git", &["init", "-b", "main"], &root)?;
    run_cmd("git", &["add", "."], &root)?;
    run_cmd(
        "git",
        &[
            "commit",
            "-m",
            "chore: initial project scaffold via runesh new",
        ],
        &root,
    )?;

    // ── GitHub repo (optional) ─────────────────────────────────────────

    if should_create_github {
        println!("  {} Creating GitHub repository...", style("->").green());
        if Command::new("gh").arg("--version").output().is_err() {
            println!(
                "  {} `gh` CLI not found. Install it: https://cli.github.com/",
                style("!").yellow()
            );
            println!("  {} Skipping GitHub repo creation.", style("!").yellow());
        } else {
            let repo_name = if let Some(ref org) = org {
                format!("{org}/{name}")
            } else {
                name.clone()
            };
            let mut args: Vec<String> = vec![
                "repo".into(),
                "create".into(),
                repo_name.clone(),
                "--description".into(),
                description.clone(),
                "--source".into(),
                ".".into(),
                "--push".into(),
            ];
            if private {
                args.push("--private".into());
            } else {
                args.push("--public".into());
            }
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            match run_cmd("gh", &arg_refs, &root) {
                Ok(_) => println!(
                    "  {} GitHub repo created: {}",
                    style("OK").green().bold(),
                    style(&repo_name).cyan()
                ),
                Err(e) => println!(
                    "  {} GitHub repo creation failed: {}",
                    style("!").yellow(),
                    e
                ),
            }
        }
    }

    // ── Done ───────────────────────────────────────────────────────────

    println!(
        "\n  {} Project '{}' ready!\n",
        style("OK").green().bold(),
        style(&name).cyan()
    );
    println!("  Structure:");
    println!("    {name}/");
    println!("    ├── Cargo.toml  package.json");
    println!("    ├── compose.yaml");
    println!("    ├── Dockerfile                (web-builder + rust-builder + runtime)");
    println!("    ├── .dockerignore  .env.example  .gitignore");
    println!("    ├── CLAUDE.md  README.md");
    println!("    ├── crates/{snake_name}-server/");
    println!("    ├── packages/web/             (Next.js 15 static SPA, Tailwind v4)");
    println!("    ├── migrations/0001_init.sql");
    println!("    └── docs/");
    println!("\n  RUNESH crates pinned to {}:", runesh_tag());
    for c in &selected_crates {
        println!("    - runesh-{c}");
    }
    println!("\n  Next steps:");
    println!("    cd {name}");
    println!("    cp .env.example .env");
    println!("    docker compose up -d --build");
    println!("    curl http://localhost:8080/healthz");
    println!();

    Ok(())
}

// ── File generation ──────────────────────────────────────────────────────────

fn generate_workspace_cargo(description: &str, crates: &BTreeSet<String>) -> String {
    let desc = crate::validate::toml_string(description);
    let tag = runesh_tag();
    let mut runesh_deps = String::new();
    for c in crates.iter() {
        let name = format!("runesh-{c}");
        let extra = runesh_feature_suffix(c);
        runesh_deps.push_str(&format!(
            "{name:<18} = {{ git = \"{repo}\", tag = \"{tag}\"{extra} }}\n",
            repo = crate::DEFAULT_REPO,
        ));
    }

    format!(
        r#"[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version     = "0.1.0"
edition     = "2024"
rust-version = "1.85"
license     = "MIT OR Apache-2.0"
description = {desc}

[workspace.dependencies]
# Runtime
tokio              = {{ version = "1", features = ["full"] }}
tokio-util         = "0.7"
async-trait        = "0.1"
futures            = "0.3"
futures-util       = "0.3"

# Tracing / errors
tracing            = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter", "fmt", "json"] }}
thiserror          = "2"
anyhow             = "1"

# Serialization
serde              = {{ version = "1", features = ["derive"] }}
serde_json         = "1"
bytes              = "1"

# Time / IDs
chrono             = {{ version = "0.4", features = ["serde"] }}
uuid               = {{ version = "1", features = ["v4", "serde"] }}

# HTTP
axum               = {{ version = "0.8", features = ["ws", "macros"] }}
axum-extra         = {{ version = "0.10", features = ["typed-header"] }}
tower              = "0.5"
tower-http         = {{ version = "0.6", features = ["cors", "trace", "compression-br", "compression-gzip", "fs"] }}
reqwest            = {{ version = "0.12", default-features = false, features = ["json", "rustls-tls"] }}

# Database
sqlx               = {{ version = "0.8", default-features = false, features = [
    "runtime-tokio", "tls-rustls", "postgres",
    "chrono", "uuid", "json", "migrate", "macros",
] }}

# Config + secrets
config             = "0.15"
secrecy            = "0.10"

# ── RUNESH (pinned) ───────────────────────────────────────────────────────
# To iterate on RUNESH alongside this repo without modifying Cargo.toml,
# write a gitignored .cargo/config.toml with a [patch] block that
# redirects these git URLs to a local checkout. Never replace these
# entries with path deps in-tree; that makes Docker builds brittle and
# hides version drift from reviewers.
{runesh_deps}"#
    )
}

/// Tack on features for crates that have useful ones.
fn runesh_feature_suffix(c: &str) -> &'static str {
    match c {
        "notify" => r#", features = ["email"]"#,
        "relay" => r#", features = ["tls"]"#,
        "telemetry" => r#", features = ["axum"]"#,
        _ => "",
    }
}

fn generate_server_cargo(snake_name: &str, crates: &BTreeSet<String>) -> String {
    let mut ws_uses = String::new();
    for dep in [
        "tokio",
        "tokio-util",
        "futures",
        "futures-util",
        "async-trait",
        "tracing",
        "tracing-subscriber",
        "thiserror",
        "anyhow",
        "serde",
        "serde_json",
        "bytes",
        "chrono",
        "uuid",
        "axum",
        "axum-extra",
        "tower",
        "tower-http",
        "reqwest",
        "sqlx",
        "config",
        "secrecy",
    ] {
        ws_uses.push_str(&format!("{dep:<18} = {{ workspace = true }}\n"));
    }
    for c in crates {
        ws_uses.push_str(&format!("runesh-{c:<10}      = {{ workspace = true }}\n"));
    }

    format!(
        r#"[package]
name        = "{snake_name}-server"
version.workspace      = true
edition.workspace      = true
license.workspace      = true
description.workspace  = true

[[bin]]
name = "{snake_name}-server"
path = "src/main.rs"

[dependencies]
{ws_uses}"#
    )
}

fn generate_server_main(snake_name: &str, _crates: &BTreeSet<String>) -> String {
    format!(
        r#"#![deny(unsafe_code)]
//! {snake_name} server entry point.
//!
//! Serves the JSON API under `/api/*` plus health probes, and falls
//! back to the static Next.js SPA bundled at `APP_WEB_DIR` (default
//! `/app/public`). One binary, one port.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use axum::{{Router, routing::get}};
use secrecy::ExposeSecret;
use serde::Deserialize;
use sqlx::postgres::{{PgPool, PgPoolOptions}};
use tower_http::services::{{ServeDir, ServeFile}};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{{EnvFilter, prelude::*}};

#[derive(Clone, Debug, Deserialize)]
struct AppConfig {{
    #[serde(default = "default_bind")]
    bind: String,
    database: DatabaseConfig,
}}

fn default_bind() -> String {{ "0.0.0.0:8080".into() }}

#[derive(Clone, Debug, Deserialize)]
struct DatabaseConfig {{
    url: secrecy::SecretString,
    #[serde(default = "default_pool")]
    pool_size: u32,
}}

fn default_pool() -> u32 {{ 10 }}

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,{snake_name}_server=debug,sqlx::query=warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg: AppConfig = ::config::Config::builder()
        .add_source(
            ::config::Environment::with_prefix("APP")
                .prefix_separator("__")
                .separator("__")
                .try_parsing(true),
        )
        .build()
        .context("config build")?
        .try_deserialize()
        .context("config deserialize")?;

    tracing::info!(bind = %cfg.bind, "starting {snake_name}-server");

    let pool: PgPool = PgPoolOptions::new()
        .max_connections(cfg.database.pool_size)
        .connect(cfg.database.url.expose_secret())
        .await
        .context("connect postgres")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("run migrations")?;

    let api = Router::new()
        .route("/healthz", get(|| async {{ "ok" }}))
        .route("/readyz", get(readyz))
        .route("/api/healthz", get(|| async {{ "ok" }}))
        .route("/api/readyz", get(readyz))
        .with_state(pool);

    let web_dir: PathBuf = std::env::var("APP_WEB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/app/public"));
    let app = if web_dir.join("index.html").exists() {{
        let index = web_dir.join("index.html");
        let spa = ServeDir::new(&web_dir).not_found_service(ServeFile::new(index));
        api.fallback_service(spa)
    }} else {{
        tracing::warn!(
            path = %web_dir.display(),
            "web bundle not found; only /api and /healthz will respond"
        );
        api
    }};
    let app = app.layer(TraceLayer::new_for_http());

    let addr: SocketAddr = cfg.bind.parse().context("parse APP__BIND")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {{addr}}"))?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("server")?;
    Ok(())
}}

async fn readyz(axum::extract::State(pool): axum::extract::State<PgPool>) -> axum::Json<serde_json::Value> {{
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
        .is_ok();
    axum::Json(serde_json::json!({{ "db": db_ok }}))
}}
"#
    )
}

fn generate_dockerfile(snake_name: &str) -> String {
    format!(
        r#"# syntax=docker/dockerfile:1.7
#
# {snake_name} server image.
#
# Single image: the Rust server serves the JSON API under /api/* and
# falls back to the static Next.js SPA built from packages/web. One
# container, one port, no reverse proxy needed.
#
#     docker compose up -d --build

# ── Stage 1: Build the static web bundle ────────────────────────────────────
FROM oven/bun:1 AS web-builder

WORKDIR /build
COPY package.json ./
COPY packages/web/package.json packages/web/package.json
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile || bun install

COPY packages/web ./packages/web
WORKDIR /build/packages/web
ENV NEXT_TELEMETRY_DISABLED=1
RUN bun run build

# ── Stage 2: Build the Rust server ──────────────────────────────────────────
FROM rust:1-bookworm AS rust-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked -p {snake_name}-server && \
    cp target/release/{snake_name}-server /{snake_name}-server

# ── Stage 3: Runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 tini \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 1000 --shell /usr/sbin/nologin app

WORKDIR /app
COPY --from=rust-builder /{snake_name}-server /usr/local/bin/{snake_name}-server
COPY migrations ./migrations
COPY --from=web-builder /build/packages/web/out ./public

USER app
ENV APP__BIND=0.0.0.0:8080 \
    RUST_LOG=info,{snake_name}_server=debug \
    APP_WEB_DIR=/app/public
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://localhost:8080/healthz || exit 1

ENTRYPOINT ["tini", "--"]
CMD ["/usr/local/bin/{snake_name}-server"]
"#
    )
}

fn generate_compose(name: &str, snake_name: &str) -> String {
    format!(
        r#"# {name} local stack. Run: docker compose up -d --build

name: {name}

services:
  db:
    image: postgres:18-alpine
    restart: unless-stopped
    environment:
      POSTGRES_DB: ${{POSTGRES_DB:-{snake_name}}}
      POSTGRES_USER: ${{POSTGRES_USER:-{snake_name}}}
      POSTGRES_PASSWORD: ${{POSTGRES_PASSWORD:-{snake_name}_dev_password}}
    command: >
      postgres
      -c shared_preload_libraries=pg_stat_statements
      -c pg_stat_statements.track=all
      -c max_connections=200
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${{POSTGRES_USER:-{snake_name}}} -d ${{POSTGRES_DB:-{snake_name}}}"]
      interval: 5s
      timeout: 5s
      retries: 10
    networks: [internal]

  server:
    build:
      context: .
      dockerfile: Dockerfile
    restart: unless-stopped
    depends_on:
      db:
        condition: service_healthy
    environment:
      APP__BIND: "0.0.0.0:8080"
      APP__DATABASE__URL: "postgres://${{POSTGRES_USER:-{snake_name}}}:${{POSTGRES_PASSWORD:-{snake_name}_dev_password}}@db:5432/${{POSTGRES_DB:-{snake_name}}}"
      APP__DATABASE__POOL_SIZE: "20"
      RUST_LOG: ${{RUST_LOG:-info,{snake_name}_server=debug}}
    ports:
      - "${{APP_PORT:-8080}}:8080"
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://localhost:8080/healthz"]
      interval: 30s
      timeout: 5s
      start_period: 10s
      retries: 3
    deploy:
      resources:
        limits:
          cpus: "2.0"
          memory: 1G
    networks: [internal]

volumes:
  pgdata:

networks:
  internal:
    driver: bridge
"#
    )
}

/// Root-level `package.json` that declares a bun workspace. Minimal;
/// just a shim so `bun install` at the repo root resolves
/// `packages/web`.
fn generate_root_package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "{name}",
  "private": true,
  "workspaces": ["packages/*"],
  "scripts": {{
    "dev:web": "bun --cwd packages/web dev",
    "build:web": "bun --cwd packages/web build"
  }},
  "packageManager": "bun@1.1"
}}
"#
    )
}

fn generate_web_package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "@{name}/web",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "dev": "next dev -p 3000",
    "build": "next build",
    "lint": "next lint",
    "typecheck": "tsc --noEmit"
  }},
  "dependencies": {{
    "@tanstack/react-query": "^5.62.0",
    "axios": "^1.7.7",
    "clsx": "^2.1.1",
    "lucide-react": "^0.460.0",
    "next": "^15.0.3",
    "next-themes": "^0.4.3",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "tailwind-merge": "^2.5.4",
    "tw-animate-css": "^1.2.5"
  }},
  "devDependencies": {{
    "@tailwindcss/postcss": "^4.0.0",
    "@types/node": "^22.9.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "eslint": "^9.15.0",
    "eslint-config-next": "^15.0.3",
    "postcss": "^8.4.49",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.6.3"
  }}
}}
"#
    )
}

const WEB_NEXT_CONFIG: &str = r#"/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Produce a static SPA in `out/`. The Rust server serves it and
  // handles `/api/*` on the same origin, so no rewrites or CORS needed.
  output: "export",
  // Directory URLs: `/foo` is served by `out/foo/index.html`. Matches
  // how tower-http ServeDir resolves directories, so hard-reloads work
  // for every route.
  trailingSlash: true,
  images: { unoptimized: true },
};

export default nextConfig;
"#;

const WEB_POSTCSS_CONFIG: &str = r#"export default {
  plugins: {
    "@tailwindcss/postcss": {},
  },
};
"#;

const WEB_TSCONFIG: &str = r#"{
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
  "include": ["next-env.d.ts", "src/**/*.ts", "src/**/*.tsx", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
"#;

/// Minimal Tailwind v4 boot. Consumers can extend with shadcn tokens
/// later; we don't pull the full RUNESH UI theme here because that
/// package currently has some broken imports that would break
/// out-of-the-box scaffolds.
const WEB_GLOBALS_CSS: &str = r#"@import "tailwindcss";
@import "tw-animate-css";

@custom-variant dark (&:is(.dark *));

:root {
  --radius: 0.625rem;
  --background: oklch(0.985 0.002 265);
  --foreground: oklch(0.145 0.012 265);
  --card: oklch(1 0 0);
  --card-foreground: oklch(0.145 0.012 265);
  --popover: oklch(1 0 0);
  --popover-foreground: oklch(0.145 0.012 265);
  --primary: oklch(0.488 0.217 264);
  --primary-foreground: oklch(0.985 0 0);
  --secondary: oklch(0.97 0 0);
  --secondary-foreground: oklch(0.205 0 0);
  --muted: oklch(0.97 0 0);
  --muted-foreground: oklch(0.556 0 0);
  --accent: oklch(0.97 0 0);
  --accent-foreground: oklch(0.205 0 0);
  --destructive: oklch(0.577 0.245 27.325);
  --border: oklch(0.922 0 0);
  --input: oklch(0.922 0 0);
  --ring: oklch(0.708 0 0);
}

.dark {
  --background: oklch(0.145 0.012 265);
  --foreground: oklch(0.985 0 0);
  --card: oklch(0.205 0.014 265);
  --card-foreground: oklch(0.985 0 0);
  --popover: oklch(0.205 0.014 265);
  --popover-foreground: oklch(0.985 0 0);
  --primary: oklch(0.6 0.2 264);
  --primary-foreground: oklch(0.985 0 0);
  --secondary: oklch(0.269 0.01 265);
  --secondary-foreground: oklch(0.985 0 0);
  --muted: oklch(0.269 0.01 265);
  --muted-foreground: oklch(0.708 0 0);
  --accent: oklch(0.269 0.01 265);
  --accent-foreground: oklch(0.985 0 0);
  --destructive: oklch(0.704 0.191 22.216);
  --border: oklch(1 0 0 / 10%);
  --input: oklch(1 0 0 / 15%);
  --ring: oklch(0.556 0 0);
  color-scheme: dark;
}

@theme inline {
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  --color-card: var(--card);
  --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover);
  --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary);
  --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary);
  --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted);
  --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent);
  --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive);
  --color-border: var(--border);
  --color-input: var(--input);
  --color-ring: var(--ring);
  --radius-sm: calc(var(--radius) * 0.6);
  --radius-md: calc(var(--radius) * 0.8);
  --radius-lg: var(--radius);
  --radius-xl: calc(var(--radius) * 1.4);
}
"#;

fn generate_web_layout(name: &str) -> String {
    let md = crate::validate::markdown_escape(name);
    format!(
        r#"import type {{ Metadata }} from "next";
import {{ AppShell }} from "@/components/app-shell";
import {{ Providers }} from "@/components/providers";
import "./globals.css";

export const metadata: Metadata = {{
  title: "{md}",
  description: "Built on RUNESH",
}};

export default function RootLayout({{ children }}: {{ children: React.ReactNode }}) {{
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <Providers>
          <AppShell>{{children}}</AppShell>
        </Providers>
      </body>
    </html>
  );
}}
"#
    )
}

fn generate_web_app_shell(name: &str) -> String {
    let md = crate::validate::markdown_escape(name);
    format!(
        r#""use client";

import Link from "next/link";
import {{ usePathname }} from "next/navigation";
import {{ Gauge, LucideIcon, Search, Sparkles }} from "lucide-react";

type NavItem = {{ title: string; href: string; icon: LucideIcon }};

const NAV: NavItem[] = [
  {{ title: "Overview", href: "/", icon: Gauge }},
];

function isActive(pathname: string, href: string): boolean {{
  const normalized = pathname.replace(/\/$/, "") || "/";
  if (href === "/") return normalized === "/";
  return normalized === href || normalized.startsWith(href + "/");
}}

export function AppShell({{ children }}: {{ children: React.ReactNode }}) {{
  const pathname = usePathname();

  return (
    <div className="flex h-screen w-full overflow-hidden bg-background text-foreground">
      <aside className="flex w-60 shrink-0 flex-col border-r border-border">
        <div className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-4">
          <Sparkles className="h-5 w-5" />
          <span className="text-base font-semibold tracking-tight">{md}</span>
        </div>

        <nav className="flex-1 overflow-y-auto p-2">
          {{NAV.map((item) => {{
            const Icon = item.icon;
            const active = isActive(pathname, item.href);
            return (
              <Link
                key={{item.href}}
                href={{item.href}}
                data-active={{active || undefined}}
                className="flex items-center gap-2.5 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted hover:text-foreground data-[active=true]:bg-muted data-[active=true]:text-foreground"
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span>{{item.title}}</span>
              </Link>
            );
          }})}}
        </nav>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center gap-4 border-b border-border px-4">
          <div className="flex h-9 max-w-md flex-1 items-center gap-2 rounded-md border border-border bg-background px-3 text-sm text-muted-foreground">
            <Search className="h-4 w-4" />
            <span className="truncate">Search...</span>
          </div>
        </header>

        <main className="flex-1 overflow-auto p-4 md:p-6">{{children}}</main>
      </div>
    </div>
  );
}}
"#
    )
}

const WEB_PAGE_HEADER: &str = r#"type PageHeaderProps = {
  title: string;
  description?: string;
  actions?: React.ReactNode;
};

export function PageHeader({ title, description, actions }: PageHeaderProps) {
  return (
    <div className="mb-6 flex flex-wrap items-start justify-between gap-4">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        {description ? (
          <p className="text-sm text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex items-center gap-2">{actions}</div> : null}
    </div>
  );
}
"#;

fn generate_web_page(_name: &str) -> String {
    r#""use client";

import { useQuery } from "@tanstack/react-query";
import { Database as DatabaseIcon, LucideIcon, Server } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { api } from "@/lib/api";

type Readyz = { db?: boolean };

export default function OverviewPage() {
  const health = useQuery<Readyz>({
    queryKey: ["readyz"],
    queryFn: () => api.get<Readyz>("/readyz").then((r) => r.data),
    refetchInterval: 10_000,
  });

  return (
    <>
      <PageHeader
        title="Overview"
        description="Live status of the control plane."
      />
      <div className="grid gap-4 sm:grid-cols-2">
        <Stat
          icon={DatabaseIcon}
          label="Database"
          value={
            health.isLoading
              ? "checking..."
              : health.data?.db
                ? "healthy"
                : "unreachable"
          }
          tone={health.data?.db ? "good" : health.isLoading ? "muted" : "bad"}
        />
        <Stat icon={Server} label="Server" value="OK" tone="good" />
      </div>
    </>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
  tone,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  tone?: "good" | "bad" | "muted";
}) {
  const toneClass =
    tone === "good"
      ? "text-emerald-500"
      : tone === "bad"
        ? "text-destructive"
        : "text-muted-foreground";
  return (
    <div className="rounded-lg border border-border bg-card p-4 shadow-sm">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Icon className="h-4 w-4" />
        {label}
      </div>
      <div className={`mt-2 text-2xl font-semibold tracking-tight ${toneClass}`}>
        {value}
      </div>
    </div>
  );
}
"#
    .to_string()
}

const WEB_PROVIDERS: &str = r#""use client";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "next-themes";
import { useState } from "react";

export function Providers({ children }: { children: React.ReactNode }) {
  const [client] = useState(() => new QueryClient());
  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </ThemeProvider>
  );
}
"#;

const WEB_API_CLIENT: &str = r#"import axios from "axios";

// Same-origin API. The server serves this SPA and the backend on the
// same port, so `/api` always resolves to the backend.
export const api = axios.create({
  baseURL: "/api",
  withCredentials: true,
  timeout: 30_000,
});

api.interceptors.response.use(undefined, (err) => {
  if (err?.response?.status === 401 && typeof window !== "undefined") {
    // TODO: redirect to login once the auth page lands.
  }
  return Promise.reject(err);
});
"#;

fn generate_local_overlay(crates: &BTreeSet<String>) -> String {
    let mut out = String::from(
        "# Gitignored overlay. `runesh new --local` drops this so\n\
         # `cargo build` uses a sibling RUNESH checkout at ../RUNESH\n\
         # instead of the pinned git tag in Cargo.toml.\n\n",
    );
    out.push_str(&format!("[patch.\"{repo}\"]\n", repo = crate::DEFAULT_REPO));
    for c in crates {
        out.push_str(&format!(
            "runesh-{c} = {{ path = \"../RUNESH/crates/runesh-{c}\" }}\n"
        ));
    }
    out
}

fn generate_claude_md(
    name: &str,
    snake_name: &str,
    description: &str,
    crates: &BTreeSet<String>,
) -> String {
    let crate_list: String = crates
        .iter()
        .map(|c| format!("- `runesh-{c}`"))
        .collect::<Vec<_>>()
        .join("\n");
    let md_name = crate::validate::markdown_escape(name);
    let md_description = crate::validate::markdown_escape(description);
    let tag = runesh_tag();

    format!(
        r#"# {md_name}

{md_description}

{base}
<!-- PROJECT -->

## Stack

- Rust (Axum) + PostgreSQL (sqlx-postgres). Postgres only; no other datastore.
- Next.js 15 static SPA (Tailwind v4) served by the Rust backend on
  the same port. One container, one port, no CORS, no cross-service
  proxy. Client pages live under `packages/web/src/app/*`.
- Shared code: RUNESH crates pinned to {tag} via git.

## Structure

```
{name}/
├── crates/
│   └── {snake_name}-server/        # Axum backend + static fallback
├── packages/web/                   # Next.js static SPA (output: "export")
│   ├── src/app/                    # pages
│   ├── src/components/providers.tsx
│   └── src/lib/api.ts              # same-origin axios client
├── migrations/                     # Postgres migrations
├── compose.yaml                    # db + server only
├── Dockerfile                      # web-builder -> rust-builder -> runtime
├── Cargo.toml                      # Rust workspace
├── package.json                    # bun workspace root
└── CLAUDE.md
```

## RUNESH crates

{crate_list}

## Commands

```bash
cp .env.example .env
docker compose up -d --build            # full stack on :8080
bun install                             # one-time web deps
bun --cwd packages/web dev              # dev web on :3000 (hot reload)
cargo run -p {snake_name}-server       # dev server (needs local Postgres)
cargo check                             # compile everything
cargo fmt --all                         # format
cargo clippy --workspace --all-targets
```

## Branch naming

`type/short-description` with prefixes `feat/`, `fix/`, `refactor/`, `chore/`, `docs/`, `test/`.

## Code conventions

- Errors via `thiserror` enums + `?`. No `anyhow::Error` in public trait methods.
- Async on Tokio; `spawn_blocking` for CPU-bound work.
- `tracing` for structured logs; no `println!` in library code.
- Secrets held in `secrecy::SecretString` / `SecretBox`, never logged.
- Postgres JSONB freely for evolving shapes; `jsonb_path_ops` GIN.
- Never introduce a path dep on RUNESH. Use git + tag.
"#,
        base = include_str!("../../../templates/CLAUDE.base.md").trim_end(),
    )
}

// ── Constants ──────────────────────────────────────────────────────────────────

const GITIGNORE: &str = r#"# Rust
target/
**/*.rs.bk
*.pdb

# Node / Bun
node_modules/
.next/
out/
dist/
bun.lock
*.tsbuildinfo

# Env + secrets
.env
.env.*
!.env.example
*.pem
*.key
credentials.json

# Local cargo overlays (path-patches for sibling RUNESH, etc.)
.cargo/

# IDE
.vscode/
.idea/
*.swp
*.swo

# OS
.DS_Store
Thumbs.db
Desktop.ini

# Tauri build output
**/gen/

# Build artifacts
*.wasm
*.dll
*.so
*.dylib

# Logs
*.log
logs/
"#;

const DOCKERIGNORE: &str = r#"target/
**/node_modules/
**/.next/
**/out/
**/dist/
**/bun.lock
**/gen/

.git/
.github/

.env
.env.*
!.env.example

.vscode/
.idea/
*.swp
.DS_Store
Thumbs.db

Dockerfile*
!Dockerfile
compose.yaml
docker-compose.yaml
"#;

const ENV_EXAMPLE: &str = r#"# Host port the server is exposed on
APP_PORT=8080

# Postgres credentials (match compose.yaml defaults)
POSTGRES_DB=
POSTGRES_USER=
POSTGRES_PASSWORD=

# Log level + module filters
RUST_LOG=info
"#;

const INITIAL_MIGRATION: &str = r#"-- Initial schema. Postgres 18+.
--
-- Extensions enabled by default:
--   pgcrypto           : gen_random_uuid, digest, hmac
--   pg_trgm            : fuzzy string search (gin_trgm_ops)
--   btree_gin          : mixed GIN indexes combining scalar + array
--   btree_gist         : range + exclusion constraints
--   citext             : case-insensitive text (emails, logins)
--   ltree              : hierarchical labels
--   hstore             : key-value bags
--   unaccent           : diacritic-insensitive search
--   pg_stat_statements : query-level observability (loaded via
--                        shared_preload_libraries in compose.yaml)

CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";
CREATE EXTENSION IF NOT EXISTS "btree_gin";
CREATE EXTENSION IF NOT EXISTS "btree_gist";
CREATE EXTENSION IF NOT EXISTS "citext";
CREATE EXTENSION IF NOT EXISTS "ltree";
CREATE EXTENSION IF NOT EXISTS "hstore";
CREATE EXTENSION IF NOT EXISTS "unaccent";
CREATE EXTENSION IF NOT EXISTS "pg_stat_statements";

-- Add your tables below.
"#;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn run_cmd(cmd: &str, args: &[&str], dir: &PathBuf) -> Result<(), String> {
    let output = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run `{cmd}`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`{cmd} {}` failed: {stderr}", args.join(" ")));
    }
    Ok(())
}
