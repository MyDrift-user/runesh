use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use console::style;
use dialoguer::{Input, MultiSelect};

mod templates;

pub fn run(
    name: Option<String>,
    repo_override: Option<String>,
    use_local: bool,
    local_path: Option<String>,
    accept_defaults: bool,
) -> Result<(), String> {
    println!("\n  {}  {}\n", style("RUNESH").bold().cyan(), style("Project Scaffolder").dim());

    // ── Determine target directory ──────────────────────────────────────

    let (root, project_name) = resolve_target_dir(name.as_deref())?;
    let snake_name = project_name.replace('-', "_");

    // ── Resolve RUNESH source ───────────────────────────────────────────

    let source = if use_local {
        RuneshSource::Local(resolve_local_path(&root, local_path)?)
    } else {
        RuneshSource::Git(
            repo_override
                .or_else(|| std::env::var("RUNESH_REPO").ok())
                .unwrap_or_else(|| crate::DEFAULT_REPO.into()),
        )
    };

    // ── Step 1: Pick components ─────────────────────────────────────────

    let (has_server, has_web, has_tauri, has_desktop_frontend, has_extension);

    if accept_defaults {
        has_server = true;
        has_web = true;
        has_tauri = false;
        has_desktop_frontend = false;
        has_extension = false;
        println!("  {} Using defaults: Rust server + Web frontend\n", style("->").green());
    } else {
        println!("  {} Select the components for your project:\n", style("1/3").dim());

        let components = &[
            "Rust API server (Axum + PostgreSQL)",
            "Web frontend (Next.js + shadcn/ui)",
            "Tauri desktop app",
            "Desktop frontend (separate from web, for Tauri)",
            "Chrome extension (WXT + React)",
        ];

        let selected = MultiSelect::new()
            .with_prompt("Components (space to toggle, enter to confirm)")
            .items(components)
            .defaults(&[true, true, false, false, false])
            .interact()
            .map_err(|e| e.to_string())?;

        if selected.is_empty() {
            return Err("No components selected".into());
        }

        has_server = selected.contains(&0);
        has_web = selected.contains(&1);
        has_tauri = selected.contains(&2);
        has_desktop_frontend = selected.contains(&3);
        has_extension = selected.contains(&4);
    }

    // ── Step 2: Pick frontend features (if web selected) ──────────────

    let mut with_editor = false;

    if has_web {
        if accept_defaults {
            with_editor = true;
        } else {
            println!("\n  {} Select frontend features:\n", style("2/4").dim());

            let features = &[
                "Novel WYSIWYG editor (wiki/rich text with tables, slash commands)",
            ];

            let sel = MultiSelect::new()
                .with_prompt("Frontend features (space to toggle)")
                .items(features)
                .defaults(&[true])
                .interact()
                .map_err(|e| e.to_string())?;

            with_editor = sel.contains(&0);
        }
    }

    // ── Step 3: Pick server features (if server selected) ───────────────

    let mut with_auth = false;
    let mut with_rate_limit = false;
    let mut with_ws = false;
    let mut with_upload = false;
    let mut with_openapi = false;
    let mut with_docker = false;

    if has_server {
        if accept_defaults {
            with_auth = true;
            with_rate_limit = true;
            with_openapi = true;
            with_docker = true;
        } else {
            println!("\n  {} Select server features:\n", style("3/4").dim());

            let features = &[
                "OIDC Authentication (runesh-auth)",
                "Rate Limiting",
                "WebSocket Broadcast",
                "File Upload Handler",
                "OpenAPI / Swagger UI (utoipa)",
                "Docker (Dockerfile + compose.yaml)",
            ];

            let sel = MultiSelect::new()
                .with_prompt("Server features (space to toggle)")
                .items(features)
                .defaults(&[true, true, false, false, true, true])
                .interact()
                .map_err(|e| e.to_string())?;

            with_auth = sel.contains(&0);
            with_rate_limit = sel.contains(&1);
            with_ws = sel.contains(&2);
            with_upload = sel.contains(&3);
            with_openapi = sel.contains(&4);
            with_docker = sel.contains(&5);
        }
    }

    // ── Step 3: Server config (if server selected) ──────────────────────

    let (db_name, port) = if has_server {
        if accept_defaults {
            (project_name.clone(), "3001".into())
        } else {
            println!("\n  {} Configure:\n", style("4/4").dim());

            let db: String = Input::new()
                .with_prompt("Database name")
                .default(project_name.clone())
                .interact_text()
                .map_err(|e| e.to_string())?;
            let p: String = Input::new()
                .with_prompt("Backend port")
                .default("3001".into())
                .interact_text()
                .map_err(|e| e.to_string())?;
            (db, p)
        }
    } else {
        (String::new(), String::new())
    };

    println!("\n  {} Creating project...\n", style("->").green());

    // ── Build config ────────────────────────────────────────────────────

    let config = ProjectConfig {
        name: project_name.clone(),
        snake_name,
        db_name,
        port,
        source,
        has_server,
        has_web,
        has_tauri,
        has_desktop_frontend,
        has_extension,
        with_auth,
        with_rate_limit,
        with_ws,
        with_upload,
        with_editor,
        with_openapi,
        with_docker,
    };

    // ── Generate ────────────────────────────────────────────────────────

    create_dirs(&root, &config)?;
    write_files(&root, &config)?;
    setup_npmrc(&root, &config)?;
    run_bun_installs(&root, &config);

    // ── Done ────────────────────────────────────────────────────────────

    println!("\n  {} Project '{}' ready!\n", style("OK").green().bold(), style(&project_name).cyan());
    println!("  Next steps:");
    if name.is_some() {
        println!("    cd {project_name}");
    }
    if has_server {
        println!();
        println!("    # Backend:");
        println!("    cargo run -p {project_name}-server");
    }
    if has_web {
        println!();
        println!("    # Web frontend:");
        println!("    cd web && bun dev");
    }
    if has_tauri && !has_desktop_frontend {
        println!();
        println!("    # Desktop (Tauri):");
        println!("    cd src-tauri && cargo tauri dev");
    }
    if has_tauri && has_desktop_frontend {
        println!();
        println!("    # Desktop frontend:");
        println!("    cd desktop && bun dev");
        println!("    # Desktop app (Tauri):");
        println!("    cd src-tauri && cargo tauri dev");
    }
    if has_extension {
        println!();
        println!("    # Chrome extension:");
        println!("    cd extension && bun dev");
        println!("    # Load: chrome://extensions -> Load unpacked -> extension/.output/chrome-mv3");
    }
    if with_docker {
        println!();
        println!("    # Docker:");
        println!("    docker compose up -d");
    }
    println!();

    Ok(())
}

// ── Types ───────────────────────────────────────────────────────────────────

pub(crate) enum RuneshSource {
    Git(String),
    Local(String),
}

pub(crate) struct ProjectConfig {
    pub name: String,
    pub snake_name: String,
    pub db_name: String,
    pub port: String,
    pub source: RuneshSource,
    // Components
    pub has_server: bool,
    pub has_web: bool,
    pub has_tauri: bool,
    pub has_desktop_frontend: bool,
    pub has_extension: bool,
    // Server features
    pub with_auth: bool,
    pub with_rate_limit: bool,
    pub with_ws: bool,
    pub with_upload: bool,
    pub with_editor: bool,
    pub with_openapi: bool,
    pub with_docker: bool,
}

impl ProjectConfig {
    pub fn cargo_dep(&self, crate_name: &str) -> String {
        match &self.source {
            RuneshSource::Git(repo) => format!("{crate_name} = {{ git = \"{repo}\" }}"),
            RuneshSource::Local(path) => format!("{crate_name} = {{ path = \"{path}/crates/{crate_name}\" }}"),
        }
    }

    pub fn cargo_dep_with_features(&self, crate_name: &str, features: &[&str]) -> String {
        let feats = features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
        match &self.source {
            RuneshSource::Git(repo) => format!("{crate_name} = {{ git = \"{repo}\", features = [{feats}] }}"),
            RuneshSource::Local(path) => format!("{crate_name} = {{ path = \"{path}/crates/{crate_name}\", features = [{feats}] }}"),
        }
    }

    pub fn npm_ui_dep(&self) -> String {
        match &self.source {
            RuneshSource::Git(_) => "\"@runesh/ui\": \"*\"".into(),
            RuneshSource::Local(path) => format!("\"@runesh/ui\": \"file:{path}/packages/ui\""),
        }
    }

    pub fn has_any_frontend(&self) -> bool {
        self.has_web || self.has_desktop_frontend || self.has_extension
    }

    pub fn has_any_rust(&self) -> bool {
        self.has_server || self.has_tauri
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn resolve_target_dir(name: Option<&str>) -> Result<(PathBuf, String), String> {
    match name {
        Some(n) => {
            let dir = PathBuf::from(n);
            if dir.exists() {
                let has_content = fs::read_dir(&dir)
                    .map_err(|e| format!("Cannot read {n}: {e}"))?
                    .any(|e| e.ok().map(|e| {
                        let s = e.file_name();
                        s != ".git" && s != ".gitattributes" && s != ".gitignore"
                    }).unwrap_or(false));
                if has_content {
                    return Err(format!("Directory '{n}' is not empty"));
                }
            }
            Ok((dir, n.to_string()))
        }
        None => {
            let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
            let has_content = fs::read_dir(&cwd)
                .map_err(|e| format!("Cannot read cwd: {e}"))?
                .any(|e| e.ok().map(|entry| {
                    !entry.file_name().to_string_lossy().starts_with(".git")
                }).unwrap_or(false));
            if has_content {
                return Err("Current directory is not empty. Use 'runesh init <name>' to create a subdirectory.".into());
            }
            let dir_name = cwd.file_name().and_then(|n| n.to_str())
                .map(|s| s.to_string()).unwrap_or_else(|| "my-app".into());
            Ok((cwd, dir_name))
        }
    }
}

fn resolve_local_path(target_dir: &Path, explicit: Option<String>) -> Result<String, String> {
    if let Some(p) = explicit {
        let path = PathBuf::from(&p);
        if path.join("Cargo.toml").exists() { return make_relative(target_dir, &path); }
        return Err(format!("RUNESH not found at '{p}'"));
    }
    if let Ok(p) = std::env::var("RUNESH_PATH") {
        let path = PathBuf::from(&p);
        if path.join("Cargo.toml").exists() { return make_relative(target_dir, &path); }
    }
    let sibling = if target_dir.is_absolute() {
        target_dir.parent().map(|p| p.join("RUNESH"))
    } else {
        std::env::current_dir().ok()
            .and_then(|cwd| cwd.join(target_dir).parent().map(|p| p.join("RUNESH")))
    };
    if let Some(ref path) = sibling {
        if path.join("Cargo.toml").exists() { return make_relative(target_dir, path); }
    }
    println!("  {} RUNESH not found locally. Defaulting to ../RUNESH", style("!").yellow());
    Ok("../RUNESH".into())
}

fn make_relative(from: &Path, to: &Path) -> Result<String, String> {
    let from_abs = if from.is_absolute() { from.to_path_buf() }
        else { std::env::current_dir().map_err(|e| e.to_string())?.join(from) };
    let to_abs = if to.is_absolute() { to.to_path_buf() }
        else { std::env::current_dir().map_err(|e| e.to_string())?.join(to) };
    let from_parts: Vec<_> = from_abs.components().collect();
    let to_parts: Vec<_> = to_abs.components().collect();
    let common = from_parts.iter().zip(to_parts.iter()).take_while(|(a, b)| a == b).count();
    if common == 0 { return Ok(to_abs.to_string_lossy().replace('\\', "/")); }
    let mut rel = String::new();
    for _ in 0..(from_parts.len() - common) { rel.push_str("../"); }
    for part in &to_parts[common..] {
        rel.push_str(&part.as_os_str().to_string_lossy());
        rel.push('/');
    }
    if rel.ends_with('/') { rel.pop(); }
    Ok(rel)
}

fn setup_npmrc(root: &Path, config: &ProjectConfig) -> Result<(), String> {
    if let RuneshSource::Git(_) = &config.source {
        let npmrc = format!("{scope}:registry=https://npm.pkg.github.com\n", scope = crate::DEFAULT_NPM_SCOPE);
        let frontends: Vec<&str> = [
            if config.has_web { Some("web") } else { None },
            if config.has_desktop_frontend { Some("desktop") } else { None },
            if config.has_extension { Some("extension") } else { None },
        ].into_iter().flatten().collect();

        for dir in frontends {
            fs::write(root.join(dir).join(".npmrc"), &npmrc)
                .map_err(|e| format!("write {dir}/.npmrc: {e}"))?;
        }
    }
    Ok(())
}

fn run_bun_installs(root: &Path, config: &ProjectConfig) {
    for (dir, label) in [
        (config.has_web, "web"),
        (config.has_desktop_frontend, "desktop"),
        (config.has_extension, "extension"),
    ] {
        if dir {
            run_bun_install(&root.join(label), label);
        }
    }
}

fn run_bun_install(dir: &Path, label: &str) {
    println!("  {} Installing {label} dependencies...", style("->").green());
    match Command::new("bun").arg("install").current_dir(dir).status() {
        Ok(s) if s.success() => {}
        Ok(_) => println!("  {} bun install had warnings in {label}/ (non-fatal)", style("!").yellow()),
        Err(_) => println!("  {} bun not found - run 'bun install' in {label}/ manually", style("!").yellow()),
    }

    // Only init shadcn for Next.js frontends, not extensions
    if label != "extension" {
        println!("  {} Initializing shadcn/ui in {label}/...", style("->").green());
        match Command::new("bunx").args(["shadcn@latest", "init", "-y", "-d"]).current_dir(dir).status() {
            Ok(s) if s.success() => {}
            _ => println!("  {} shadcn init skipped in {label}/", style("!").yellow()),
        }
    }
}

fn create_dirs(root: &Path, c: &ProjectConfig) -> Result<(), String> {
    let mk = |d: &str| fs::create_dir_all(root.join(d)).map_err(|e| format!("mkdir {d}: {e}"));

    if c.has_server {
        mk("crates")?;
        mk(&format!("crates/{}-server/src", c.name))?;
        mk("migrations")?;
    }
    if c.has_web {
        for d in &["web/src/app", "web/src/components", "web/src/lib", "web/public"] { mk(d)?; }
    }
    if c.has_tauri {
        for d in &["src-tauri/src", "src-tauri/icons", "src-tauri/capabilities"] { mk(d)?; }
    }
    if c.has_desktop_frontend {
        for d in &["desktop/src/app", "desktop/src/components", "desktop/src/lib", "desktop/public"] { mk(d)?; }
        if c.has_server {
            mk(&format!("crates/{}-desktop/src", c.name))?;
        }
    }
    if c.has_extension {
        for d in &["extension/entrypoints/popup", "extension/public"] { mk(d)?; }
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

    w(".gitignore", templates::GITIGNORE)?;

    // ── Rust workspace ──────────────────────────────────────────────────

    if c.has_any_rust() || c.has_server {
        w("Cargo.toml", &templates::cargo_workspace(c))?;
    }
    if c.has_server {
        w(".env", &templates::dot_env(c))?;
        let sc = format!("crates/{}-server", c.name);
        w(&format!("{sc}/Cargo.toml"), &templates::server_cargo(c))?;
        w(&format!("{sc}/src/main.rs"), &templates::server_main(c))?;
        w("migrations/001_initial.sql", &templates::initial_migration(c))?;
    }

    // ── Web frontend ────────────────────────────────────────────────────

    if c.has_web {
        w("web/package.json", &templates::web_package_json(c))?;
        w("web/tsconfig.json", templates::TSCONFIG)?;
        w("web/next.config.ts", templates::NEXT_CONFIG)?;
        w("web/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
        w("web/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
        w("web/src/app/layout.tsx", &templates::layout_tsx(c, false))?;
        w("web/src/app/page.tsx", &templates::home_page(c))?;
        w("web/src/lib/utils.ts", templates::UTILS_TS)?;

        if c.with_editor {
            w("web/src/app/editor/page.tsx", &templates::editor_page(c))?;
            w("web/src/components/editor.tsx", templates::EDITOR_COMPONENT)?;
        }
    }

    // ── Docker ──────────────────────────────────────────────────────────

    if c.with_docker {
        w("Dockerfile", &templates::dockerfile(c))?;
        w("compose.yaml", &templates::compose_yaml(c))?;
    }

    // ── Tauri ───────────────────────────────────────────────────────────

    if c.has_tauri {
        if c.has_desktop_frontend {
            if c.has_server {
                let dc = format!("crates/{}-desktop", c.name);
                w(&format!("{dc}/Cargo.toml"), &templates::desktop_backend_cargo(c))?;
                w(&format!("{dc}/src/lib.rs"), &templates::desktop_backend_lib(c))?;
            }
            w("desktop/package.json", &templates::desktop_package_json(c))?;
            w("desktop/tsconfig.json", templates::TSCONFIG)?;
            w("desktop/next.config.ts", templates::NEXT_CONFIG_STATIC)?;
            w("desktop/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
            w("desktop/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
            w("desktop/src/app/layout.tsx", &templates::layout_tsx(c, true))?;
            w("desktop/src/app/page.tsx", &templates::desktop_home_page(c))?;
            w("desktop/src/lib/utils.ts", templates::UTILS_TS)?;
            w("src-tauri/Cargo.toml", &templates::tauri_cargo_separate(c))?;
            w("src-tauri/tauri.conf.json", &templates::tauri_conf_separate(c))?;
        } else {
            w("src-tauri/Cargo.toml", &templates::tauri_cargo(c))?;
            w("src-tauri/tauri.conf.json", &templates::tauri_conf(c))?;
        }
        w("src-tauri/build.rs", "fn main() { tauri_build::build(); }\n")?;
        w("src-tauri/src/main.rs", &templates::tauri_main(c))?;
        w("src-tauri/src/lib.rs", &templates::tauri_lib(c))?;
        w("src-tauri/capabilities/default.json", templates::TAURI_CAPABILITIES)?;
    }

    // ── Chrome Extension ────────────────────────────────────────────────

    if c.has_extension {
        w("extension/package.json", &templates::extension_package_json(c))?;
        w("extension/wxt.config.ts", &templates::extension_wxt_config(c))?;
        w("extension/tsconfig.json", templates::EXTENSION_TSCONFIG)?;
        w("extension/postcss.config.js", templates::EXTENSION_POSTCSS)?;
        w("extension/entrypoints/popup/index.html", &templates::extension_popup_html(c))?;
        w("extension/entrypoints/popup/main.tsx", templates::EXTENSION_POPUP_MAIN)?;
        w("extension/entrypoints/popup/App.tsx", &templates::extension_popup_app(c))?;
        w("extension/entrypoints/popup/style.css", templates::EXTENSION_POPUP_CSS)?;
        w("extension/entrypoints/background.ts", templates::EXTENSION_BACKGROUND)?;
    }

    // ── CLAUDE.md ───────────────────────────────────────────────────────

    w("CLAUDE.md", &templates::claude_md(c))?;

    Ok(())
}
