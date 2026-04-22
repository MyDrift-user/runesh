//! `runesh new` — Create a new project repo wired to RUNESH.
//!
//! Produces a modern, deployable skeleton:
//!
//! - Cargo workspace pulling runesh-* crates via `git + tag` pinned to
//!   the CLI's own version, never path refs.
//! - `crates/{snake}-server` with Axum + sqlx-postgres + config-via-env.
//! - `Dockerfile` multi-stage (tini, non-root, buildkit cache mounts).
//! - `compose.yaml` (modern spec, no top-level `version:`,
//!   `name: {project}`, Postgres + server with healthchecks).
//! - `migrations/0001_init.sql` seeded with useful Postgres extensions.
//! - `.env.example`, `.dockerignore`, sensible `.gitignore`.
//! - `CLAUDE.md` + `README.md`.

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
    println!("    ├── Cargo.toml");
    println!("    ├── compose.yaml");
    println!("    ├── Dockerfile");
    println!("    ├── .dockerignore  .env.example  .gitignore");
    println!("    ├── CLAUDE.md  README.md");
    println!("    ├── crates/{snake_name}-server/");
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
tower-http         = {{ version = "0.6", features = ["cors", "trace", "compression-br", "compression-gzip"] }}
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

use std::net::SocketAddr;

use anyhow::Context;
use axum::{{Router, routing::get}};
use secrecy::ExposeSecret;
use serde::Deserialize;
use sqlx::postgres::{{PgPool, PgPoolOptions}};
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

    let app = Router::new()
        .route("/healthz", get(|| async {{ "ok" }}))
        .route("/readyz", get(readyz))
        .with_state(pool)
        .layer(TraceLayer::new_for_http());

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
# Single-context build. RUNESH is pulled via git+tag from Cargo.toml,
# so no sibling checkout is needed. Build with:
#     docker compose up -d --build

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

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 tini \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 1000 --shell /usr/sbin/nologin app

WORKDIR /app
COPY --from=rust-builder /{snake_name}-server /usr/local/bin/{snake_name}-server
COPY migrations ./migrations

USER app
ENV APP__BIND=0.0.0.0:8080 \
    RUST_LOG=info,{snake_name}_server=debug
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
- Shared code: RUNESH crates pinned to {tag} via git.

## Structure

```
{name}/
├── crates/
│   └── {snake_name}-server/        # Axum backend
├── migrations/                     # Postgres migrations
├── compose.yaml                    # docker compose stack
├── Dockerfile                      # Multi-stage server build
├── Cargo.toml                      # Rust workspace
└── CLAUDE.md
```

## RUNESH crates

{crate_list}

## Commands

```bash
cp .env.example .env
docker compose up -d --build        # stack on :8080
cargo run -p {snake_name}-server   # dev run (needs local Postgres)
cargo check                         # compile everything
cargo fmt --all                     # format
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
