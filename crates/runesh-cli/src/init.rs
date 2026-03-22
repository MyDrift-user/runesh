use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use console::style;
use dialoguer::{Input, MultiSelect, Select};

mod templates;

pub fn run(name: Option<String>) -> Result<(), String> {
    println!("\n  {}  {}\n", style("RUNESH").bold().cyan(), style("Project Scaffolder").dim());

    // ── Gather config ───────────────────────────────────────────────────

    let project_name: String = match name {
        Some(n) => n,
        None => Input::new()
            .with_prompt("Project name")
            .interact_text()
            .map_err(|e| e.to_string())?,
    };

    let snake_name = project_name.replace('-', "_");

    let project_type = Select::new()
        .with_prompt("Project type")
        .items(&[
            "Web only (Rust API + Next.js)",
            "Web + Desktop (shared backend, Tauri wraps the web frontend)",
            "Web + Desktop (separate backends and separate frontends)",
        ])
        .default(0)
        .interact()
        .map_err(|e| e.to_string())?;

    let with_tauri = project_type >= 1;
    let separate_desktop = project_type == 2;

    let feature_options = &[
        "OIDC Authentication (runesh-auth)",
        "Rate Limiting",
        "WebSocket Broadcast",
        "File Upload Handler",
        "Docker (Dockerfile + compose.yaml)",
    ];

    let selected_features = MultiSelect::new()
        .with_prompt("Features to include (space to toggle)")
        .items(feature_options)
        .defaults(&[true, true, false, false, true])
        .interact()
        .map_err(|e| e.to_string())?;

    let with_auth = selected_features.contains(&0);
    let with_rate_limit = selected_features.contains(&1);
    let with_ws = selected_features.contains(&2);
    let with_upload = selected_features.contains(&3);
    let with_docker = selected_features.contains(&4);

    let db_name: String = Input::new()
        .with_prompt("Database name")
        .default(project_name.clone())
        .interact_text()
        .map_err(|e| e.to_string())?;

    let port: String = Input::new()
        .with_prompt("Web backend port")
        .default("3001".into())
        .interact_text()
        .map_err(|e| e.to_string())?;

    // Resolve RUNESH path relative to the project
    let runesh_rel = detect_runesh_path(&project_name)?;

    println!("\n  {} Creating project...\n", style("->").green());

    // ── Create directory structure ──────────────────────────────────────

    let root = PathBuf::from(&project_name);
    if root.exists() {
        return Err(format!("Directory '{}' already exists", project_name));
    }

    let config = ProjectConfig {
        name: project_name.clone(),
        snake_name: snake_name.clone(),
        db_name,
        port,
        runesh_rel: runesh_rel.clone(),
        with_auth,
        with_rate_limit,
        with_ws,
        with_upload,
        with_tauri,
        separate_desktop,
        with_docker,
    };

    create_dirs(&root, &config)?;
    write_files(&root, &config)?;

    // ── Run bun install ────────────────────────────────────────────────

    let web_dir = root.join("web");
    run_bun_install(&web_dir, "web");

    if config.separate_desktop {
        let desktop_web_dir = root.join("desktop");
        run_bun_install(&desktop_web_dir, "desktop");
    }

    // ── Done ───────────────────────────────────────────────────────────

    println!("\n  {} Project '{}' created!\n", style("OK").green().bold(), style(&project_name).cyan());
    println!("  Next steps:");
    println!("    cd {project_name}");
    println!();
    println!("    # Web backend:");
    println!("    cargo run -p {project_name}-server");
    println!("    # Web frontend:");
    println!("    cd web && bun dev");
    if with_tauri && !separate_desktop {
        println!();
        println!("    # Desktop (wraps web frontend):");
        println!("    cd src-tauri && cargo tauri dev");
    }
    if separate_desktop {
        println!();
        println!("    # Desktop app (separate frontend + backend):");
        println!("    cd desktop && bun dev   # frontend on :3100");
        println!("    cd src-tauri && cargo tauri dev");
    }
    if with_docker {
        println!();
        println!("    # Docker (web only):");
        println!("    docker compose up -d");
    }
    println!();

    Ok(())
}

pub(crate) struct ProjectConfig {
    pub name: String,
    pub snake_name: String,
    pub db_name: String,
    pub port: String,
    pub runesh_rel: String,
    pub with_auth: bool,
    pub with_rate_limit: bool,
    pub with_ws: bool,
    pub with_upload: bool,
    pub with_tauri: bool,
    pub separate_desktop: bool,
    pub with_docker: bool,
}

fn detect_runesh_path(project_name: &str) -> Result<String, String> {
    let runesh_path = PathBuf::from("../RUNESH");
    if runesh_path.join("Cargo.toml").exists() {
        return Ok("../RUNESH".into());
    }
    let from_project = PathBuf::from(project_name).join("../RUNESH");
    if from_project.join("Cargo.toml").exists() {
        return Ok("../RUNESH".into());
    }
    println!("  {} RUNESH repo not found at ../RUNESH - using relative path anyway", style("!").yellow());
    Ok("../RUNESH".into())
}

fn run_bun_install(dir: &Path, label: &str) {
    println!("  {} Installing {} dependencies with bun...", style("->").green(), label);
    let result = Command::new("bun")
        .arg("install")
        .current_dir(dir)
        .status();

    match result {
        Ok(s) if s.success() => {}
        Ok(_) => println!("  {} bun install had warnings in {label}/ (non-fatal)", style("!").yellow()),
        Err(_) => println!("  {} bun not found - run 'bun install' in {label}/ manually", style("!").yellow()),
    }

    // Initialize shadcn
    println!("  {} Initializing shadcn/ui in {label}/...", style("->").green());
    let shadcn = Command::new("bunx")
        .args(["shadcn@latest", "init", "-y", "-d"])
        .current_dir(dir)
        .status();

    match shadcn {
        Ok(s) if s.success() => {}
        _ => println!("  {} shadcn init skipped in {label}/ - run 'bunx shadcn@latest init' manually", style("!").yellow()),
    }
}

fn create_dirs(root: &Path, config: &ProjectConfig) -> Result<(), String> {
    let server_src = format!("crates/{}-server/src", config.name);
    let dirs: Vec<&str> = vec![
        "crates",
        &server_src,
        "migrations",
        "web/src/app",
        "web/src/components",
        "web/src/lib",
        "web/public",
    ];

    for d in &dirs {
        fs::create_dir_all(root.join(d))
            .map_err(|e| format!("Failed to create {d}: {e}"))?;
    }

    if config.with_tauri {
        fs::create_dir_all(root.join("src-tauri/src"))
            .map_err(|e| format!("Failed to create src-tauri: {e}"))?;
        fs::create_dir_all(root.join("src-tauri/icons"))
            .map_err(|e| format!("Failed to create icons: {e}"))?;
        fs::create_dir_all(root.join("src-tauri/capabilities"))
            .map_err(|e| format!("Failed to create capabilities: {e}"))?;
    }

    if config.separate_desktop {
        // Separate desktop frontend
        for d in &["desktop/src/app", "desktop/src/components", "desktop/src/lib", "desktop/public"] {
            fs::create_dir_all(root.join(d))
                .map_err(|e| format!("Failed to create {d}: {e}"))?;
        }
        // Separate desktop backend crate
        let desktop_crate = format!("crates/{}-desktop/src", config.name);
        fs::create_dir_all(root.join(&desktop_crate))
            .map_err(|e| format!("Failed to create {desktop_crate}: {e}"))?;
    }

    Ok(())
}

fn write_files(root: &Path, c: &ProjectConfig) -> Result<(), String> {
    let w = |path: &str, content: &str| -> Result<(), String> {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {path}: {e}"))?;
        }
        fs::write(&full, content).map_err(|e| format!("write {path}: {e}"))
    };

    // ── Root ────────────────────────────────────────────────────────────

    w("Cargo.toml", &templates::cargo_workspace(c))?;
    w(".gitignore", templates::GITIGNORE)?;
    w(".env", &templates::dot_env(c))?;

    // ── Web backend server crate ────────────────────────────────────────

    let server_crate = format!("crates/{}-server", c.name);
    w(&format!("{server_crate}/Cargo.toml"), &templates::server_cargo(c))?;
    w(&format!("{server_crate}/src/main.rs"), &templates::server_main(c))?;

    // ── Migrations ──────────────────────────────────────────────────────

    w("migrations/001_initial.sql", &templates::initial_migration(c))?;

    // ── Web frontend ────────────────────────────────────────────────────

    w("web/package.json", &templates::web_package_json(c))?;
    w("web/tsconfig.json", templates::TSCONFIG)?;
    w("web/next.config.ts", templates::NEXT_CONFIG)?;
    w("web/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
    w("web/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
    w("web/src/app/layout.tsx", &templates::layout_tsx(c, false))?;
    w("web/src/app/page.tsx", &templates::home_page(c))?;
    w("web/src/lib/utils.ts", templates::UTILS_TS)?;

    // ── Docker ──────────────────────────────────────────────────────────

    if c.with_docker {
        w("Dockerfile", &templates::dockerfile(c))?;
        w("compose.yaml", &templates::compose_yaml(c))?;
    }

    // ── Tauri (shared or separate) ──────────────────────────────────────

    if c.with_tauri {
        if c.separate_desktop {
            // Separate desktop backend crate
            let desktop_crate = format!("crates/{}-desktop", c.name);
            w(&format!("{desktop_crate}/Cargo.toml"), &templates::desktop_backend_cargo(c))?;
            w(&format!("{desktop_crate}/src/lib.rs"), &templates::desktop_backend_lib(c))?;

            // Separate desktop frontend (different Next.js app)
            w("desktop/package.json", &templates::desktop_package_json(c))?;
            w("desktop/tsconfig.json", templates::TSCONFIG)?;
            w("desktop/next.config.ts", templates::NEXT_CONFIG_STATIC)?;
            w("desktop/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
            w("desktop/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
            w("desktop/src/app/layout.tsx", &templates::layout_tsx(c, true))?;
            w("desktop/src/app/page.tsx", &templates::desktop_home_page(c))?;
            w("desktop/src/lib/utils.ts", templates::UTILS_TS)?;

            // Tauri points to desktop/ frontend
            w("src-tauri/Cargo.toml", &templates::tauri_cargo_separate(c))?;
            w("src-tauri/tauri.conf.json", &templates::tauri_conf_separate(c))?;
        } else {
            // Tauri wraps the web frontend
            w("src-tauri/Cargo.toml", &templates::tauri_cargo(c))?;
            w("src-tauri/tauri.conf.json", &templates::tauri_conf(c))?;
        }

        w("src-tauri/build.rs", "fn main() { tauri_build::build(); }\n")?;
        w("src-tauri/src/main.rs", &templates::tauri_main(c))?;
        w("src-tauri/src/lib.rs", &templates::tauri_lib(c))?;
        w("src-tauri/capabilities/default.json", templates::TAURI_CAPABILITIES)?;
    }

    // ── CLAUDE.md ───────────────────────────────────────────────────────

    w("CLAUDE.md", &templates::claude_md(c))?;

    Ok(())
}
