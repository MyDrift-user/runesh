//! `runesh new` — Create a new project repo with RUNESH integration.
//!
//! Creates a Cargo workspace with selected RUNESH crate dependencies,
//! a CLAUDE.md with project conventions, a .gitignore, git init,
//! and optionally a GitHub repo via the `gh` CLI.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use console::style;
use dialoguer::{Confirm, Input, MultiSelect};

/// All available RUNESH crates that can be included as dependencies.
const AVAILABLE_CRATES: &[(&str, &str)] = &[
    (
        "core",
        "AppError, rate limiter, WS broadcast, file upload, middleware, metrics",
    ),
    ("auth", "OIDC + JWT + RBAC + Axum middleware"),
    (
        "inventory",
        "Cross-platform hardware/software inventory collection",
    ),
    (
        "remote",
        "Remote file explorer + PTY terminal over WebSocket",
    ),
    (
        "desktop",
        "Remote desktop with screen capture, input injection, multi-cursor",
    ),
    (
        "vfs",
        "Virtual filesystem with cloud provider + overlay writes",
    ),
    ("tun", "Cross-platform TUN device (WireGuard tunneling)"),
    (
        "telemetry",
        "Sentry/GlitchTip error reporting (OPTIONAL — off by default; no-op without DSN)",
    ),
];

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

    // ── Validate name ──────────────────────────────────────────────────

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
        format!("{} — powered by RUNESH", name)
    } else {
        Input::new()
            .with_prompt("Project description")
            .default(format!("{} — powered by RUNESH", name))
            .interact_text()
            .map_err(|e| e.to_string())?
    };

    // ── Select RUNESH crates ───────────────────────────────────────────

    let selected_crates: BTreeSet<String> = if let Some(crates_str) = crates_arg {
        crates_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if accept_defaults {
        // Default: core + auth
        ["core", "auth"].iter().map(|s| s.to_string()).collect()
    } else {
        println!(
            "  {} Select RUNESH crates to include:\n",
            style("1/2").dim()
        );

        let labels: Vec<String> = AVAILABLE_CRATES
            .iter()
            .map(|(name, desc)| format!("runesh-{name}  ({desc})"))
            .collect();

        let defaults: Vec<bool> = AVAILABLE_CRATES
            .iter()
            .map(|(name, _)| *name == "core" || *name == "auth")
            .collect();

        let selected = MultiSelect::new()
            .with_prompt("Crates (space to toggle, enter to confirm)")
            .items(&labels)
            .defaults(&defaults)
            .interact()
            .map_err(|e| e.to_string())?;

        selected
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

    // ── Resolve RUNESH path ────────────────────────────────────────────

    let runesh_rel_path = if use_local {
        // Look for RUNESH as sibling directory
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let sibling = cwd.join("RUNESH");
        if sibling.join("Cargo.toml").exists() {
            "../RUNESH".to_string()
        } else {
            println!(
                "  {} RUNESH not found at ../RUNESH, using default",
                style("!").yellow()
            );
            "../RUNESH".to_string()
        }
    } else {
        String::new()
    };

    println!(
        "\n  {} Creating project '{}'...\n",
        style("->").green(),
        style(&name).cyan()
    );

    // ── Create directory structure ─────────────────────────────────────

    fs::create_dir_all(
        root.join("crates")
            .join(format!("{snake_name}-server"))
            .join("src"),
    )
    .map_err(|e| format!("mkdir: {e}"))?;
    fs::create_dir_all(root.join("docs")).map_err(|e| format!("mkdir docs: {e}"))?;

    // ── Write Cargo.toml (workspace) ───────────────────────────────────

    let cargo_toml = generate_cargo_toml(
        &name,
        &snake_name,
        &description,
        &selected_crates,
        use_local,
        &runesh_rel_path,
    );
    fs::write(root.join("Cargo.toml"), cargo_toml).map_err(|e| format!("write Cargo.toml: {e}"))?;

    // ── Write server crate ─────────────────────────────────────────────

    let server_cargo = generate_server_cargo(
        &name,
        &snake_name,
        &selected_crates,
        use_local,
        &runesh_rel_path,
    );
    fs::write(
        root.join("crates")
            .join(format!("{snake_name}-server"))
            .join("Cargo.toml"),
        server_cargo,
    )
    .map_err(|e| format!("write server Cargo.toml: {e}"))?;

    let server_main = generate_server_main(&snake_name, &selected_crates);
    fs::write(
        root.join("crates")
            .join(format!("{snake_name}-server"))
            .join("src")
            .join("main.rs"),
        server_main,
    )
    .map_err(|e| format!("write main.rs: {e}"))?;

    // ── Write .gitignore ───────────────────────────────────────────────

    fs::write(root.join(".gitignore"), GITIGNORE).map_err(|e| format!("write .gitignore: {e}"))?;

    // ── Write CLAUDE.md ────────────────────────────────────────────────

    let claude_md = generate_claude_md(&name, &description, &selected_crates);
    fs::write(root.join("CLAUDE.md"), claude_md).map_err(|e| format!("write CLAUDE.md: {e}"))?;

    // ── Write README.md ────────────────────────────────────────────────

    let md_name = crate::validate::markdown_escape(&name);
    let md_description = crate::validate::markdown_escape(&description);
    let readme = format!(
        "# {md_name}\n\n{md_description}\n\nBuilt with [RUNESH](https://github.com/MyDrift-user/runesh).\n"
    );
    fs::write(root.join("README.md"), readme).map_err(|e| format!("write README.md: {e}"))?;

    // ── Git init ───────────────────────────────────────────────────────

    println!("  {} Initializing git repository...", style("->").green());
    run_cmd("git", &["init"], &root)?;
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

        // Check if gh CLI is available
        if Command::new("gh").arg("--version").output().is_err() {
            println!(
                "  {} `gh` CLI not found. Install it: https://cli.github.com/",
                style("!").yellow()
            );
            println!("  {} Skipping GitHub repo creation.", style("!").yellow());
        } else {
            let mut args = vec!["repo".to_string(), "create".to_string()];

            // Repo name (with optional org prefix)
            let repo_name = if let Some(ref org) = org {
                format!("{org}/{name}")
            } else {
                name.clone()
            };
            args.push(repo_name.clone());

            args.push("--description".into());
            args.push(description.clone());
            args.push("--source".into());
            args.push(".".into());
            args.push("--push".into());

            if private {
                args.push("--private".into());
            } else {
                args.push("--public".into());
            }

            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            match run_cmd("gh", &arg_refs, &root) {
                Ok(_) => {
                    println!(
                        "  {} GitHub repo created: {}",
                        style("OK").green().bold(),
                        style(&repo_name).cyan()
                    );
                }
                Err(e) => {
                    println!(
                        "  {} GitHub repo creation failed: {}",
                        style("!").yellow(),
                        e
                    );
                    println!(
                        "  {} You can create it manually later.",
                        style("!").yellow()
                    );
                }
            }
        }
    }

    // ── Done ───────────────────────────────────────────────────────────

    println!(
        "\n  {} Project '{}' ready!\n",
        style("OK").green().bold(),
        style(&name).cyan()
    );

    println!("  Project structure:");
    println!("    {name}/");
    println!("    ├── Cargo.toml              # Workspace root");
    println!("    ├── CLAUDE.md               # AI assistant instructions");
    println!("    ├── .gitignore");
    println!("    ├── README.md");
    println!("    ├── crates/");
    println!("    │   └── {snake_name}-server/  # Your Axum server");
    println!("    └── docs/");

    println!("\n  RUNESH crates included:");
    for c in &selected_crates {
        println!("    - runesh-{c}");
    }

    println!("\n  Next steps:");
    println!("    cd {name}");
    println!("    cargo check");
    println!("    cargo run -p {snake_name}-server");
    println!();

    Ok(())
}

// ── File Generation ──────────────────────────────────────────────────────────

fn generate_cargo_toml(
    _name: &str,
    _snake_name: &str,
    description: &str,
    _crates: &BTreeSet<String>,
    _use_local: bool,
    _runesh_path: &str,
) -> String {
    let desc_toml = crate::validate::toml_string(description);
    format!(
        r#"[workspace]
resolver = "2"
members = [
    "crates/*",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
description = {desc_toml}

[workspace.dependencies]
tokio = {{ version = "1", features = ["full"] }}
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter"] }}
bytes = "1"
thiserror = "2"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
    )
}

fn generate_server_cargo(
    _name: &str,
    snake_name: &str,
    crates: &BTreeSet<String>,
    use_local: bool,
    runesh_path: &str,
) -> String {
    let mut deps = String::new();
    deps.push_str("tokio = { workspace = true }\n");
    deps.push_str("tracing = { workspace = true }\n");
    deps.push_str("tracing-subscriber = { workspace = true }\n");
    deps.push_str("serde = { workspace = true }\n");
    deps.push_str("serde_json = { workspace = true }\n");
    deps.push_str("axum = { version = \"0.8\", features = [\"ws\"] }\n");
    deps.push_str("tower-http = { version = \"0.6\", features = [\"cors\", \"trace\"] }\n");

    deps.push('\n');
    deps.push_str("# RUNESH shared crates\n");

    for c in crates.iter() {
        let crate_name = format!("runesh-{c}");
        if use_local {
            deps.push_str(&format!(
                "{crate_name} = {{ path = \"{runesh_path}/crates/{crate_name}\" }}\n"
            ));
        } else {
            deps.push_str(&format!(
                "{crate_name} = {{ git = \"{}\" }}\n",
                crate::DEFAULT_REPO
            ));
        }
    }

    format!(
        r#"[package]
name = "{snake_name}-server"
version.workspace = true
edition.workspace = true

[[bin]]
name = "{snake_name}-server"
path = "src/main.rs"

[dependencies]
{deps}"#
    )
}

fn generate_server_main(snake_name: &str, crates: &BTreeSet<String>) -> String {
    let mut imports = String::new();
    let mut setup = String::new();

    if crates.contains("core") {
        imports.push_str("use runesh_core::AppError;\n");
        setup.push_str("    // runesh-core: middleware, rate limiting, WS broadcast\n");
    }
    if crates.contains("auth") {
        imports.push_str("// use runesh_auth::axum_middleware;\n");
        setup.push_str("    // runesh-auth: OIDC/JWT middleware available\n");
    }

    format!(
        r#"//! {snake_name} server entry point.

use axum::{{Router, routing::get}};
use tower_http::trace::TraceLayer;
{imports}
#[tokio::main]
async fn main() {{
    tracing_subscriber::fmt()
        .with_env_filter("info,{snake_name}=debug")
        .init();

{setup}
    let app = Router::new()
        .route("/health", get(|| async {{ "ok" }}))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    tracing::info!("listening on http://0.0.0.0:3001");
    axum::serve(listener, app).await.unwrap();
}}
"#
    )
}

fn generate_claude_md(name: &str, description: &str, crates: &BTreeSet<String>) -> String {
    let crate_list: String = crates
        .iter()
        .map(|c| format!("- `runesh-{c}`"))
        .collect::<Vec<_>>()
        .join("\n");
    let md_name = crate::validate::markdown_escape(name);
    let md_description = crate::validate::markdown_escape(description);

    format!(
        r#"# {md_name}

{md_description}

{base}
<!-- PROJECT -->

## Stack

- Rust (Axum) + PostgreSQL (SQLx)
- Next.js + React 19 + shadcn/ui v4 + Tailwind CSS v4
- Package manager: bun
- Shared code: @mydrift/runesh-ui + runesh crates

## Structure

```
{name}/
├── crates/
│   └── {snake_name}-server/     # Axum backend server
├── web/                         # Next.js frontend
├── Cargo.toml                   # Rust workspace
├── CLAUDE.md                    # This file
```

## RUNESH Integration

This project uses [RUNESH](https://github.com/MyDrift-user/runesh) shared crates:

{crate_list}

## Commands

```bash
cargo check                      # Check compilation
cargo build                      # Build debug
cargo run -p {snake_name}-server # Run server
cargo test                       # Run tests
cd web && bun dev                # Start frontend
docker compose up -d             # Start full stack
```

## Branch Naming

`type/short-description`

Prefixes: `feat/`, `fix/`, `refactor/`, `chore/`, `docs/`, `test/`

## Code Conventions

- **Error handling**: `thiserror` enums with `?` operator
- **Async**: tokio runtime, `spawn_blocking` for CPU work
- **Logging**: `tracing` crate with structured fields
- **Serialization**: `serde` with `#[serde(rename_all = "snake_case")]`
"#,
        base = include_str!("../../../templates/CLAUDE.base.md").trim_end(),
        snake_name = name.replace('-', "_"),
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

# Environment & Secrets
.env
.env.*
!.env.example
*.pem
*.key
credentials.json

# IDE
.vscode/
.idea/
*.swp
*.swo

# OS
.DS_Store
Thumbs.db
Desktop.ini

# Build Artifacts
*.wasm
*.dll
*.so
*.dylib

# Logs
*.log
logs/

# Misc
*.bak
*.tmp
*.orig
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
