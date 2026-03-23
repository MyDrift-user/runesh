use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use console::style;
use dialoguer::{Input, MultiSelect, Select};

mod templates;

pub fn run(
    name: Option<String>,
    repo_override: Option<String>,
    use_local: bool,
    local_path: Option<String>,
) -> Result<(), String> {
    println!("\n  {}  {}\n", style("RUNESH").bold().cyan(), style("Project Scaffolder").dim());

    // ── Determine target directory ──────────────────────────────────────

    let (root, project_name) = match name {
        Some(ref n) => {
            let dir = PathBuf::from(n);
            if dir.exists() {
                let has_content = fs::read_dir(&dir)
                    .map_err(|e| format!("Cannot read {n}: {e}"))?
                    .any(|e| {
                        e.ok()
                            .map(|e| e.file_name() != ".git" && e.file_name() != ".gitattributes")
                            .unwrap_or(false)
                    });
                if has_content {
                    return Err(format!("Directory '{n}' is not empty"));
                }
            }
            (dir, n.clone())
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("Cannot get current directory: {e}"))?;

            let has_content = fs::read_dir(&cwd)
                .map_err(|e| format!("Cannot read current directory: {e}"))?
                .any(|e| {
                    e.ok()
                        .map(|entry| {
                            let s = entry.file_name().to_string_lossy().to_string();
                            !s.starts_with(".git")
                        })
                        .unwrap_or(false)
                });
            if has_content {
                return Err(
                    "Current directory is not empty. Use 'runesh init <name>' to create a subdirectory, or run in an empty directory.".into()
                );
            }

            let dir_name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "my-app".into());

            (cwd, dir_name)
        }
    };

    let snake_name = project_name.replace('-', "_");

    // ── Resolve RUNESH source ───────────────────────────────────────────

    let source = if use_local {
        let path = resolve_local_path(&root, local_path)?;
        RuneshSource::Local(path)
    } else {
        let repo = repo_override
            .or_else(|| std::env::var("RUNESH_REPO").ok())
            .unwrap_or_else(|| crate::DEFAULT_REPO.into());
        RuneshSource::Git(repo)
    };

    // ── Gather config ───────────────────────────────────────────────────

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

    println!("\n  {} Creating project...\n", style("->").green());

    let config = ProjectConfig {
        name: project_name.clone(),
        snake_name: snake_name.clone(),
        db_name,
        port,
        source,
        with_auth: selected_features.contains(&0),
        with_rate_limit: selected_features.contains(&1),
        with_ws: selected_features.contains(&2),
        with_upload: selected_features.contains(&3),
        with_tauri,
        separate_desktop,
        with_docker: selected_features.contains(&4),
    };

    create_dirs(&root, &config)?;
    write_files(&root, &config)?;

    // ── Setup .npmrc for GitHub Packages ────────────────────────────────

    if let RuneshSource::Git(_) = &config.source {
        let npmrc_content = format!(
            "{scope}:registry=https://npm.pkg.github.com\n",
            scope = crate::DEFAULT_NPM_SCOPE
        );
        let web_npmrc = root.join("web").join(".npmrc");
        fs::write(&web_npmrc, &npmrc_content)
            .map_err(|e| format!("write .npmrc: {e}"))?;

        if config.separate_desktop {
            let desktop_npmrc = root.join("desktop").join(".npmrc");
            fs::write(&desktop_npmrc, &npmrc_content)
                .map_err(|e| format!("write desktop/.npmrc: {e}"))?;
        }
    }

    // ── Run bun install ────────────────────────────────────────────────

    let web_dir = root.join("web");
    run_bun_install(&web_dir, "web");

    if config.separate_desktop {
        run_bun_install(&root.join("desktop"), "desktop");
    }

    // ── Done ───────────────────────────────────────────────────────────

    println!("\n  {} Project '{}' ready!\n", style("OK").green().bold(), style(&project_name).cyan());
    println!("  Next steps:");
    if name.is_some() {
        println!("    cd {project_name}");
        println!();
    }
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
        println!("    # Desktop app:");
        println!("    cd desktop && bun dev   # frontend on :3100");
        println!("    cd src-tauri && cargo tauri dev");
    }
    if config.with_docker {
        println!();
        println!("    # Docker:");
        println!("    docker compose up -d");
    }
    println!();

    Ok(())
}

// ── Types ───────────────────────────────────────────────────────────────────

pub(crate) enum RuneshSource {
    /// Git URL (e.g. "https://github.com/USER/RUNESH")
    Git(String),
    /// Local file path (e.g. "../RUNESH")
    Local(String),
}

pub(crate) struct ProjectConfig {
    pub name: String,
    pub snake_name: String,
    pub db_name: String,
    pub port: String,
    pub source: RuneshSource,
    pub with_auth: bool,
    pub with_rate_limit: bool,
    pub with_ws: bool,
    pub with_upload: bool,
    pub with_tauri: bool,
    pub separate_desktop: bool,
    pub with_docker: bool,
}

impl ProjectConfig {
    /// Cargo dependency string for a RUNESH crate.
    pub fn cargo_dep(&self, crate_name: &str) -> String {
        match &self.source {
            RuneshSource::Git(repo) => {
                format!("{crate_name} = {{ git = \"{repo}\" }}")
            }
            RuneshSource::Local(path) => {
                format!("{crate_name} = {{ path = \"{path}/crates/{crate_name}\" }}")
            }
        }
    }

    /// npm dependency string for @runesh/ui.
    pub fn npm_ui_dep(&self) -> String {
        match &self.source {
            RuneshSource::Git(_) => format!("\"@runesh/ui\": \"*\""),
            RuneshSource::Local(path) => format!("\"@runesh/ui\": \"file:{path}/packages/ui\""),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn resolve_local_path(target_dir: &Path, explicit: Option<String>) -> Result<String, String> {
    if let Some(p) = explicit {
        let path = PathBuf::from(&p);
        if path.join("Cargo.toml").exists() {
            return make_relative(target_dir, &path);
        }
        return Err(format!("RUNESH not found at '{p}'"));
    }

    if let Ok(p) = std::env::var("RUNESH_PATH") {
        let path = PathBuf::from(&p);
        if path.join("Cargo.toml").exists() {
            return make_relative(target_dir, &path);
        }
    }

    // Sibling directory
    let sibling = if target_dir.is_absolute() {
        target_dir.parent().map(|p| p.join("RUNESH"))
    } else {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.join(target_dir).parent().map(|p| p.join("RUNESH")))
    };

    if let Some(ref path) = sibling {
        if path.join("Cargo.toml").exists() {
            return make_relative(target_dir, path);
        }
    }

    println!("  {} RUNESH repo not found locally. Defaulting to ../RUNESH", style("!").yellow());
    Ok("../RUNESH".into())
}

fn make_relative(from: &Path, to: &Path) -> Result<String, String> {
    let from_abs = if from.is_absolute() {
        from.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(from)
    };
    let to_abs = if to.is_absolute() {
        to.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(to)
    };

    let from_parts: Vec<_> = from_abs.components().collect();
    let to_parts: Vec<_> = to_abs.components().collect();

    let common = from_parts.iter().zip(to_parts.iter())
        .take_while(|(a, b)| a == b).count();

    if common == 0 {
        return Ok(to_abs.to_string_lossy().replace('\\', "/"));
    }

    let ups = from_parts.len() - common;
    let mut rel = String::new();
    for _ in 0..ups {
        rel.push_str("../");
    }
    for part in &to_parts[common..] {
        rel.push_str(&part.as_os_str().to_string_lossy());
        rel.push('/');
    }
    if rel.ends_with('/') { rel.pop(); }

    Ok(rel)
}

fn run_bun_install(dir: &Path, label: &str) {
    println!("  {} Installing {} dependencies with bun...", style("->").green(), label);
    let result = Command::new("bun").arg("install").current_dir(dir).status();

    match result {
        Ok(s) if s.success() => {}
        Ok(_) => println!("  {} bun install had warnings in {label}/ (non-fatal)", style("!").yellow()),
        Err(_) => println!("  {} bun not found - run 'bun install' in {label}/ manually", style("!").yellow()),
    }

    println!("  {} Initializing shadcn/ui in {label}/...", style("->").green());
    let shadcn = Command::new("bunx")
        .args(["shadcn@latest", "init", "-y", "-d"])
        .current_dir(dir)
        .status();

    match shadcn {
        Ok(s) if s.success() => {}
        _ => println!("  {} shadcn init skipped in {label}/", style("!").yellow()),
    }
}

fn create_dirs(root: &Path, config: &ProjectConfig) -> Result<(), String> {
    let server_src = format!("crates/{}-server/src", config.name);
    let dirs: Vec<&str> = vec!["crates", &server_src, "migrations",
        "web/src/app", "web/src/components", "web/src/lib", "web/public"];

    for d in &dirs {
        fs::create_dir_all(root.join(d)).map_err(|e| format!("mkdir {d}: {e}"))?;
    }

    if config.with_tauri {
        for d in &["src-tauri/src", "src-tauri/icons", "src-tauri/capabilities"] {
            fs::create_dir_all(root.join(d)).map_err(|e| format!("mkdir {d}: {e}"))?;
        }
    }

    if config.separate_desktop {
        for d in &["desktop/src/app", "desktop/src/components", "desktop/src/lib", "desktop/public"] {
            fs::create_dir_all(root.join(d)).map_err(|e| format!("mkdir {d}: {e}"))?;
        }
        let desktop_crate = format!("crates/{}-desktop/src", config.name);
        fs::create_dir_all(root.join(&desktop_crate)).map_err(|e| format!("mkdir {desktop_crate}: {e}"))?;
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

    w("Cargo.toml", &templates::cargo_workspace(c))?;
    w(".gitignore", templates::GITIGNORE)?;
    w(".env", &templates::dot_env(c))?;

    let sc = format!("crates/{}-server", c.name);
    w(&format!("{sc}/Cargo.toml"), &templates::server_cargo(c))?;
    w(&format!("{sc}/src/main.rs"), &templates::server_main(c))?;
    w("migrations/001_initial.sql", &templates::initial_migration(c))?;

    w("web/package.json", &templates::web_package_json(c))?;
    w("web/tsconfig.json", templates::TSCONFIG)?;
    w("web/next.config.ts", templates::NEXT_CONFIG)?;
    w("web/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
    w("web/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
    w("web/src/app/layout.tsx", &templates::layout_tsx(c, false))?;
    w("web/src/app/page.tsx", &templates::home_page(c))?;
    w("web/src/lib/utils.ts", templates::UTILS_TS)?;

    if c.with_docker {
        w("Dockerfile", &templates::dockerfile(c))?;
        w("compose.yaml", &templates::compose_yaml(c))?;
    }

    if c.with_tauri {
        if c.separate_desktop {
            let dc = format!("crates/{}-desktop", c.name);
            w(&format!("{dc}/Cargo.toml"), &templates::desktop_backend_cargo(c))?;
            w(&format!("{dc}/src/lib.rs"), &templates::desktop_backend_lib(c))?;
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

    w("CLAUDE.md", &templates::claude_md(c))?;

    Ok(())
}
