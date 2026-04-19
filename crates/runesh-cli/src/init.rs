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
    println!(
        "\n  {}  {}\n",
        style("RUNESH").bold().cyan(),
        style("Project Scaffolder").dim()
    );

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
        println!(
            "  {} Using defaults: Rust server + Web frontend\n",
            style("->").green()
        );
    } else {
        println!(
            "  {} Select the components for your project:\n",
            style("1/3").dim()
        );

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

    let mut with_dashboard = false;
    let mut with_editor = false;
    let mut with_data_table = false;
    let mut with_telemetry_web = false;

    if has_web {
        if accept_defaults {
            with_dashboard = true;
            with_editor = true;
            with_data_table = true;
            // telemetry stays opt-in even with --yes
        } else {
            println!("\n  {} Select frontend features:\n", style("2/4").dim());

            let features = &[
                "Dashboard shell (sidebar + toolbar + search command palette)",
                "Novel WYSIWYG editor (wiki/rich text, file attachments, tables)",
                "Data table (sortable, paginated, searchable)",
                "Error reporting via Sentry/GlitchTip (OPTIONAL — off by default; deploy without a DSN to disable)",
            ];

            let sel = MultiSelect::new()
                .with_prompt("Frontend features (space to toggle)")
                .items(features)
                .defaults(&[true, true, true, false])
                .interact()
                .map_err(|e| e.to_string())?;

            with_dashboard = sel.contains(&0);
            with_editor = sel.contains(&1);
            with_data_table = sel.contains(&2);
            with_telemetry_web = sel.contains(&3);
        }
    }

    // ── Step 3: Pick server features (if server selected) ───────────────

    let mut with_auth = false;
    let mut with_rate_limit = false;
    let mut with_ws = false;
    let mut with_upload = false;
    let mut with_openapi = false;
    let mut with_docker = false;
    let mut with_telemetry_server = false;

    if has_server {
        if accept_defaults {
            with_auth = true;
            with_rate_limit = true;
            with_openapi = true;
            with_docker = true;
            // telemetry stays opt-in even with --yes
        } else {
            println!("\n  {} Select server features:\n", style("3/4").dim());

            let features = &[
                "OIDC Authentication (runesh-auth)",
                "Rate Limiting",
                "WebSocket Broadcast",
                "File Upload Handler",
                "OpenAPI / Swagger UI (utoipa)",
                "Docker (Dockerfile + compose.yaml)",
                "Error reporting via Sentry/GlitchTip (OPTIONAL — off by default; deploy without a DSN to disable)",
            ];

            let sel = MultiSelect::new()
                .with_prompt("Server features (space to toggle)")
                .items(features)
                .defaults(&[true, true, false, false, true, true, false])
                .interact()
                .map_err(|e| e.to_string())?;

            with_auth = sel.contains(&0);
            with_rate_limit = sel.contains(&1);
            with_ws = sel.contains(&2);
            with_upload = sel.contains(&3);
            with_openapi = sel.contains(&4);
            with_docker = sel.contains(&5);
            with_telemetry_server = sel.contains(&6);
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
        with_dashboard,
        with_editor,
        with_data_table,
        with_openapi,
        with_docker,
        with_telemetry_server,
        with_telemetry_web,
    };

    // ── Generate ────────────────────────────────────────────────────────

    create_dirs(&root, &config)?;
    write_files(&root, &config)?;
    run_bun_installs(&root, &config);

    // ── Done ────────────────────────────────────────────────────────────

    println!(
        "\n  {} Project '{}' ready!\n",
        style("OK").green().bold(),
        style(&project_name).cyan()
    );
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
        println!(
            "    # Load: chrome://extensions -> Load unpacked -> extension/.output/chrome-mv3"
        );
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
    pub with_dashboard: bool,
    pub with_editor: bool,
    pub with_data_table: bool,
    pub with_openapi: bool,
    pub with_docker: bool,
    /// Sentry/GlitchTip error reporting on the Rust server. Optional — off by default.
    /// Even when scaffolded in, the integration is a no-op at runtime unless
    /// `RUNESH_SENTRY_DSN` is set, so deploying without telemetry just works.
    pub with_telemetry_server: bool,
    /// Sentry/GlitchTip error reporting on the Next.js frontend. Optional — off
    /// by default. Same runtime behavior: no DSN means no reports.
    pub with_telemetry_web: bool,
}

impl ProjectConfig {
    fn repo_url(&self) -> &str {
        match &self.source {
            RuneshSource::Git(repo) => repo,
            RuneshSource::Local(_) => crate::DEFAULT_REPO,
        }
    }

    pub fn cargo_dep(&self, crate_name: &str) -> String {
        // Always use git deps - works in Docker, CI, and locally.
        // For local RUNESH iteration, use .cargo/config.toml path overrides.
        format!("{crate_name} = {{ git = \"{}\" }}", self.repo_url())
    }

    pub fn cargo_dep_with_features(&self, crate_name: &str, features: &[&str]) -> String {
        let feats = features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{crate_name} = {{ git = \"{}\", features = [{feats}] }}",
            self.repo_url()
        )
    }

    /// npm dependency for @mydrift/runesh-ui.
    /// `subdir_depth` is how many levels deep the package.json is from the project root
    /// (e.g. 1 for `web/package.json`, 1 for `extension/package.json`).
    pub fn npm_ui_dep(&self) -> String {
        self.npm_ui_dep_from_depth(1)
    }

    pub fn npm_ui_dep_from_depth(&self, _depth: usize) -> String {
        match &self.source {
            // --local: omit the dep from package.json entirely. The package
            // is satisfied by a directory junction created by
            // relink_runesh_ui() before bun install runs, so webpack/Next
            // resolves `@mydrift/runesh-ui/*` imports without bun
            // ever trying to fetch (which would 401 against GitHub Packages)
            // or copy (which EPERMs on Windows for the deep tree).
            //
            // Returning an empty string here means the consumer of this
            // function (e.g. web_package_json) must handle the empty case
            // and not emit a stray comma.
            RuneshSource::Local(_) => String::new(),
            // --git (default): public npm registry version.
            RuneshSource::Git(_) => "\"@mydrift/runesh-ui\": \"*\"".into(),
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
                    .any(|e| {
                        e.ok()
                            .map(|e| {
                                let s = e.file_name();
                                s != ".git" && s != ".gitattributes" && s != ".gitignore"
                            })
                            .unwrap_or(false)
                    });
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
                .any(|e| {
                    e.ok()
                        .map(|entry| !entry.file_name().to_string_lossy().starts_with(".git"))
                        .unwrap_or(false)
                });
            if has_content {
                return Err("Current directory is not empty. Use 'runesh init <name>' to create a subdirectory.".into());
            }
            let dir_name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "my-app".into());
            Ok((cwd, dir_name))
        }
    }
}

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
    println!(
        "  {} RUNESH not found locally. Defaulting to ../RUNESH",
        style("!").yellow()
    );
    Ok("../RUNESH".into())
}

fn make_relative(from: &Path, to: &Path) -> Result<String, String> {
    let from_abs = if from.is_absolute() {
        from.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(from)
    };
    let to_abs = if to.is_absolute() {
        to.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(to)
    };
    let from_parts: Vec<_> = from_abs.components().collect();
    let to_parts: Vec<_> = to_abs.components().collect();
    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return Ok(to_abs.to_string_lossy().replace('\\', "/"));
    }
    let mut rel = String::new();
    for _ in 0..(from_parts.len() - common) {
        rel.push_str("../");
    }
    for part in &to_parts[common..] {
        rel.push_str(&part.as_os_str().to_string_lossy());
        rel.push('/');
    }
    if rel.ends_with('/') {
        rel.pop();
    }
    Ok(rel)
}

fn run_bun_installs(root: &Path, config: &ProjectConfig) {
    for (dir, label) in [
        (config.has_web, "web"),
        (config.has_desktop_frontend, "desktop"),
        (config.has_extension, "extension"),
    ] {
        if dir {
            // Under --local the package.json doesn't list @mydrift/runesh-ui
            // (npm_ui_dep_from_depth returns "") because bun's `file:` install
            // hits EPERM on Windows for the deep tree. Pre-create a directory
            // junction (Windows) / symlink (unix) so the package is resolvable
            // BEFORE bun install runs and BEFORE shadcn add tries to read it.
            if label != "extension" {
                if let RuneshSource::Local(runesh_path) = &config.source {
                    relink_runesh_ui(&root.join(label), runesh_path);
                }
            }
            run_bun_install(&root.join(label), label);
            // bun install may have stomped the junction if it found a stub
            // package.json reference. Re-link to be safe.
            if label != "extension" {
                if let RuneshSource::Local(runesh_path) = &config.source {
                    relink_runesh_ui(&root.join(label), runesh_path);
                }
            }
        }
    }
}

/// Replace `<pkg>/node_modules/@mydrift/runesh-ui` with a junction/symlink
/// pointing at the live RUNESH packages/ui dir.
fn relink_runesh_ui(pkg_dir: &Path, runesh_relative: &str) {
    let target_link = pkg_dir.join("node_modules/@mydrift/runesh-ui");
    let mut runesh_abs: PathBuf = if Path::new(runesh_relative).is_absolute() {
        PathBuf::from(runesh_relative)
    } else {
        // runesh_relative was computed by make_relative() relative to the
        // project root (parent of pkg_dir). Resolve it now.
        pkg_dir
            .parent()
            .map(|p| p.join(runesh_relative))
            .unwrap_or_else(|| PathBuf::from(runesh_relative))
    };
    runesh_abs = runesh_abs.join("packages").join("ui");
    let runesh_abs = match fs::canonicalize(&runesh_abs) {
        Ok(p) => p,
        Err(_) => {
            println!(
                "  {} Could not resolve {} for runesh-ui junction",
                style("!").yellow(),
                runesh_abs.display()
            );
            return;
        }
    };

    // Make sure the parent dir exists.
    if let Some(parent) = target_link.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Always replace what's there — bun may have left an empty/partial copy.
    let _ = remove_dir_or_link(&target_link);

    let result = create_dir_link(&runesh_abs, &target_link);
    if result.is_ok() {
        println!(
            "  {} Linked @mydrift/runesh-ui -> RUNESH/packages/ui",
            style("OK").green()
        );
    } else {
        println!(
            "  {} Could not link runesh-ui (manual `bun install` needed): {}",
            style("!").yellow(),
            result.err().unwrap()
        );
    }
}

#[cfg(windows)]
fn create_dir_link(src: &Path, dst: &Path) -> Result<(), String> {
    // Use `mklink /J` for a directory junction. Junctions don't need admin
    // and survive across drives but only on the local machine.
    let src_str = src.to_string_lossy().replace('/', "\\");
    let dst_str = dst.to_string_lossy().replace('/', "\\");
    let status = Command::new("cmd")
        .args(["/c", "mklink", "/J", &dst_str, &src_str])
        .status()
        .map_err(|e| format!("mklink spawn: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("mklink exit code {:?}", status.code()))
    }
}

#[cfg(unix)]
fn create_dir_link(src: &Path, dst: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(src, dst).map_err(|e| format!("symlink: {e}"))
}

fn remove_dir_or_link(path: &Path) -> Result<(), String> {
    // On Windows a junction is removable via remove_dir; an empty dir is too.
    // For a populated dir we need remove_dir_all.
    if path.exists() || fs::symlink_metadata(path).is_ok() {
        fs::remove_dir_all(path)
            .or_else(|_| fs::remove_dir(path))
            .or_else(|_| fs::remove_file(path))
            .map_err(|e| format!("remove {path:?}: {e}"))?;
    }
    Ok(())
}

/// Copy a component from the RUNESH package source into the consumer project.
/// The component uses @/ imports that resolve in the consumer's build context.
fn copy_runesh_component(
    web_root: &Path,
    relative_path: &str,
    source: &RuneshSource,
) -> Result<(), String> {
    let src_path = match source {
        RuneshSource::Local(path) => PathBuf::from(path)
            .join("packages/ui/src")
            .join(relative_path),
        RuneshSource::Git(_) => {
            // For git source, try to find the linked package in node_modules
            web_root
                .join("node_modules/@mydrift/runesh-ui/src")
                .join(relative_path)
        }
    };

    let dest_path = web_root.join("src").join(relative_path);
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    if src_path.exists() {
        fs::copy(&src_path, &dest_path).map_err(|e| format!("copy {relative_path}: {e}"))?;
    } else {
        println!(
            "  {} Could not find {} - skipping",
            console::style("!").yellow(),
            relative_path
        );
    }
    Ok(())
}

fn run_bun_install(dir: &Path, label: &str) {
    println!(
        "  {} Installing {label} dependencies...",
        style("->").green()
    );
    match Command::new("bun").arg("install").current_dir(dir).status() {
        Ok(s) if s.success() => {}
        Ok(_) => println!(
            "  {} bun install had warnings in {label}/ (non-fatal)",
            style("!").yellow()
        ),
        Err(_) => {
            println!(
                "  {} bun not found - run 'bun install' in {label}/ manually",
                style("!").yellow()
            );
            return;
        }
    }

    // Only Next.js frontends need shadcn components — extensions don't.
    if label == "extension" {
        return;
    }

    // shadcn add invokes the package manager to install peer deps for
    // certain components (e.g. cmdk for `command`). On React 19 projects
    // some of those resolve via npm with peer-dep conflicts even though
    // bun would handle them. Pre-install the known troublemakers via bun
    // so shadcn add never has to touch npm.
    let preinstall = ["cmdk"];
    println!(
        "  {} Pre-installing shadcn peer deps in {label}/...",
        style("->").green()
    );
    let _ = Command::new("bun")
        .arg("add")
        .args(&preinstall)
        .current_dir(dir)
        .status();

    // Comprehensive list of components used across RUNESH dashboard, editor,
    // and data-table features. shadcn writes the .tsx file even when its
    // internal package-install step warns, so we tolerate non-zero exits.
    // components.json is pre-written by write_files, so we don't run init.
    let shadcn_components = [
        "alert-dialog",
        "avatar",
        "badge",
        "button",
        "card",
        "collapsible",
        "command",
        "dialog",
        "dropdown-menu",
        "form",
        "input",
        "label",
        "popover",
        "scroll-area",
        "select",
        "separator",
        "sheet",
        "sidebar",
        "skeleton",
        "table",
        "textarea",
        "tooltip",
    ];
    println!(
        "  {} Adding {} shadcn components in {label}/...",
        style("->").green(),
        shadcn_components.len()
    );
    match Command::new("bunx")
        .arg("--bun")
        .arg("shadcn@latest")
        .arg("add")
        .args(&shadcn_components)
        .arg("--yes")
        .arg("--overwrite")
        .current_dir(dir)
        .status()
    {
        Ok(s) if s.success() => {}
        _ => println!(
            "  {} shadcn add finished with warnings in {label}/ - some components may need manual install",
            style("!").yellow()
        ),
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
        for d in &[
            "web/src/app",
            "web/src/components",
            "web/src/lib",
            "web/public",
        ] {
            mk(d)?;
        }
    }
    if c.has_tauri {
        for d in &["src-tauri/src", "src-tauri/icons", "src-tauri/capabilities"] {
            mk(d)?;
        }
    }
    if c.has_desktop_frontend {
        for d in &[
            "desktop/src/app",
            "desktop/src/components",
            "desktop/src/lib",
            "desktop/public",
        ] {
            mk(d)?;
        }
        if c.has_server {
            mk(&format!("crates/{}-desktop/src", c.name))?;
        }
    }
    if c.has_extension {
        for d in &["extension/entrypoints/popup", "extension/public"] {
            mk(d)?;
        }
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
        w(
            "migrations/001_initial.sql",
            &templates::initial_migration(c),
        )?;
    }

    // ── Web frontend ────────────────────────────────────────────────────

    if c.has_web {
        w("web/package.json", &templates::web_package_json(c))?;
        w("web/tsconfig.json", templates::TSCONFIG)?;
        w("web/components.json", templates::COMPONENTS_JSON)?;
        w("web/next.config.ts", &templates::next_config(c))?;
        w("web/postcss.config.mjs", templates::POSTCSS_CONFIG)?;

        // ── Sentry / GlitchTip frontend integration (optional) ──
        if c.with_telemetry_web {
            w(
                "web/sentry.client.config.ts",
                templates::SENTRY_CLIENT_CONFIG,
            )?;
            w(
                "web/sentry.server.config.ts",
                templates::SENTRY_SERVER_CONFIG,
            )?;
            w("web/sentry.edge.config.ts", templates::SENTRY_EDGE_CONFIG)?;
            w(
                "web/src/instrumentation.ts",
                templates::SENTRY_INSTRUMENTATION,
            )?;
            w("web/.env.local.example", templates::sentry_web_env())?;
        }
        // Copy globals.css from RUNESH (includes theme + editor styles)
        {
            let css_src = match &c.source {
                RuneshSource::Local(path) => {
                    std::path::PathBuf::from(path).join("packages/ui/src/styles/globals.css")
                }
                RuneshSource::Git(_) => std::path::PathBuf::new(),
            };
            let css_dest = root.join("web/src/app/globals.css");
            if css_src.exists() {
                fs::copy(&css_src, &css_dest).map_err(|e| format!("copy globals.css: {e}"))?;
            } else {
                w("web/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
            }
        }
        w("web/src/app/layout.tsx", &templates::layout_tsx(c, false))?;
        w("web/src/app/page.tsx", &templates::home_page(c))?;
        w("web/src/lib/utils.ts", templates::UTILS_TS)?;
        // use-mobile hook (needed by shadcn sidebar)
        w("web/src/hooks/use-mobile.ts", templates::USE_MOBILE)?;

        // Copy RUNESH layout components (they use @/ imports for shadcn)
        if c.with_dashboard {
            w("web/src/components/app-shell.tsx", &templates::app_shell(c))?;
            copy_runesh_component(
                &root.join("web"),
                "components/layout/app-sidebar.tsx",
                &c.source,
            )?;
            copy_runesh_component(
                &root.join("web"),
                "components/layout/dashboard-shell.tsx",
                &c.source,
            )?;
            copy_runesh_component(
                &root.join("web"),
                "components/layout/page-header.tsx",
                &c.source,
            )?;
            copy_runesh_component(
                &root.join("web"),
                "components/layout/search-bar.tsx",
                &c.source,
            )?;
            copy_runesh_component(&root.join("web"), "components/ui/data-table.tsx", &c.source)?;
            copy_runesh_component(
                &root.join("web"),
                "components/ui/confirm-dialog.tsx",
                &c.source,
            )?;
        }

        // Novel WYSIWYG editor
        if c.with_editor {
            w("web/src/app/editor/page.tsx", &templates::editor_page(c))?;
            w("web/src/components/editor.tsx", templates::EDITOR_COMPONENT)?;
        }

        // Data table example page
        if c.with_data_table {
            w(
                "web/src/app/examples/page.tsx",
                &templates::data_table_page(c),
            )?;
        }
    }

    // ── Docker ──────────────────────────────────────────────────────────

    if c.with_docker {
        w("Dockerfile", &templates::dockerfile(c))?;
        w("compose.yaml", &templates::compose_yaml(c))?;
        w(".dockerignore", templates::DOCKERIGNORE)?;
    }

    // ── Tauri ───────────────────────────────────────────────────────────

    if c.has_tauri {
        if c.has_desktop_frontend {
            if c.has_server {
                let dc = format!("crates/{}-desktop", c.name);
                w(
                    &format!("{dc}/Cargo.toml"),
                    &templates::desktop_backend_cargo(c),
                )?;
                w(
                    &format!("{dc}/src/lib.rs"),
                    &templates::desktop_backend_lib(c),
                )?;
            }
            w("desktop/package.json", &templates::desktop_package_json(c))?;
            w("desktop/tsconfig.json", templates::TSCONFIG)?;
            w("desktop/next.config.ts", templates::NEXT_CONFIG_STATIC)?;
            w("desktop/postcss.config.mjs", templates::POSTCSS_CONFIG)?;
            w("desktop/src/app/globals.css", templates::GLOBALS_CSS_IMPORT)?;
            w(
                "desktop/src/app/layout.tsx",
                &templates::layout_tsx(c, true),
            )?;
            w("desktop/src/app/page.tsx", &templates::desktop_home_page(c))?;
            w("desktop/src/lib/utils.ts", templates::UTILS_TS)?;
            w("src-tauri/Cargo.toml", &templates::tauri_cargo_separate(c))?;
            w(
                "src-tauri/tauri.conf.json",
                &templates::tauri_conf_separate(c),
            )?;
        } else {
            w("src-tauri/Cargo.toml", &templates::tauri_cargo(c))?;
            w("src-tauri/tauri.conf.json", &templates::tauri_conf(c))?;
        }
        w(
            "src-tauri/build.rs",
            "fn main() { tauri_build::build(); }\n",
        )?;
        w("src-tauri/src/main.rs", &templates::tauri_main(c))?;
        w("src-tauri/src/lib.rs", &templates::tauri_lib(c))?;
        w(
            "src-tauri/capabilities/default.json",
            templates::TAURI_CAPABILITIES,
        )?;
    }

    // ── Chrome Extension ────────────────────────────────────────────────

    if c.has_extension {
        w(
            "extension/package.json",
            &templates::extension_package_json(c),
        )?;
        w(
            "extension/wxt.config.ts",
            &templates::extension_wxt_config(c),
        )?;
        w("extension/tsconfig.json", templates::EXTENSION_TSCONFIG)?;
        w("extension/postcss.config.js", templates::EXTENSION_POSTCSS)?;
        w(
            "extension/entrypoints/popup/index.html",
            &templates::extension_popup_html(c),
        )?;
        w(
            "extension/entrypoints/popup/main.tsx",
            templates::EXTENSION_POPUP_MAIN,
        )?;
        w(
            "extension/entrypoints/popup/App.tsx",
            &templates::extension_popup_app(c),
        )?;
        w(
            "extension/entrypoints/popup/style.css",
            templates::EXTENSION_POPUP_CSS,
        )?;
        w(
            "extension/entrypoints/background.ts",
            templates::EXTENSION_BACKGROUND,
        )?;
    }

    // ── CLAUDE.md ───────────────────────────────────────────────────────

    w("CLAUDE.md", &templates::claude_md(c))?;

    // ── Serena config ────────────────────────────────────────────────────

    w(".serena/project.yml", &templates::serena_config(c))?;

    Ok(())
}
