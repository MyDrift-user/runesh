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
clap = {{ version = "4", features = ["derive", "env"] }}

# RUNESH shared crates
{core_dep}
"#, core_dep = if c.with_openapi {
        c.cargo_dep_with_features("runesh-core", &["openapi"])
    } else {
        c.cargo_dep("runesh-core")
    });

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
clap = {{ version = "4", features = ["derive", "env"] }}
runesh-core.workspace = true
"#, name = c.name);

    if c.with_auth {
        deps.push_str("runesh-auth.workspace = true\n");
    }
    if c.with_openapi {
        deps.push_str("utoipa = { version = \"5\", features = [\"chrono\", \"uuid\", \"axum_extras\"] }\n");
        deps.push_str("utoipa-axum = \"0.2\"\n");
    }
    if c.with_upload || c.with_editor {
        deps.push_str("mime_guess = \"2\"\n");
    }

    deps
}

pub fn server_main(c: &ProjectConfig) -> String {
    let mut extra_cli_fields = String::new();
    let mut extra_imports = String::new();
    let mut extra_middleware = String::new();
    let mut extra_setup = String::new();

    if c.with_rate_limit {
        extra_imports.push_str("use runesh_core::rate_limit::{RateLimiter, rate_limit_layer};\n");
        extra_middleware.push_str(r#"        .layer(middleware::from_fn(move |req, next| {
            let limiter = RateLimiter::new(100, 60);
            rate_limit_layer(limiter, true, req, next)
        }))
"#);
    }
    if c.with_auth {
        extra_imports.push_str("use runesh_auth::axum_middleware::{auth_middleware, JwtSecret, AuthExemptPaths};\n");
        extra_cli_fields.push_str(r#"
    /// JWT signing secret (min 32 chars)
    #[arg(long, env = "JWT_SECRET")]
    jwt_secret: String,
"#);
        extra_middleware.push_str(r#"        .layer(middleware::from_fn(auth_middleware))
        .layer(axum::Extension(JwtSecret(cli.jwt_secret.clone())))
        .layer(axum::Extension(AuthExemptPaths(vec![
            "/auth/".into(),
            "/api/v1/health".into(),
            "/swagger-ui".into(),
            "/api/openapi.json".into(),
            "/api/uploads".into(),
        ])))
"#);
    }
    if c.with_openapi {
        extra_imports.push_str("use utoipa::OpenApi;\nuse runesh_core::openapi::{setup_swagger, SwaggerConfig, add_bearer_security};\n");
        extra_cli_fields.push_str(r#"
    /// Enable Swagger UI
    #[arg(long, env = "SWAGGER_ENABLED", default_value = "true")]
    swagger: bool,
"#);
        extra_setup.push_str(r#"
    // OpenAPI / Swagger UI
    let app = if cli.swagger {
        let mut doc = ApiDoc::openapi();
        add_bearer_security(&mut doc);
        setup_swagger(app, doc, SwaggerConfig::from_env())
    } else {
        app
    };
"#);
    }

    let mut extra_routes = String::new();
    let mut extra_handlers = String::new();

    if c.with_upload || c.with_editor {
        extra_imports.push_str("use axum::extract::Multipart;\n");
        extra_routes.push_str("        .route(\"/api/uploads\", post(upload_file))\n        .route(\"/api/uploads/{filename}\", get(serve_upload))\n");
        extra_handlers.push_str(r#"
async fn upload_file(mut multipart: Multipart) -> Result<Json<serde_json::Value>, runesh_core::AppError> {
    while let Some(field) = multipart.next_field().await.map_err(|e| runesh_core::AppError::BadRequest(e.to_string()))? {
        let uploaded = runesh_core::upload::save_upload(field, std::path::Path::new("./uploads"), 50 * 1024 * 1024, None).await?;
        return Ok(Json(serde_json::json!({
            "url": format!("/api/uploads/{}", uploaded.storage_key),
            "filename": uploaded.filename,
            "size": uploaded.size,
        })));
    }
    Err(runesh_core::AppError::BadRequest("No file provided".into()))
}

async fn serve_upload(axum::extract::Path(filename): axum::extract::Path<String>) -> Result<axum::response::Response, runesh_core::AppError> {
    let safe_name = std::path::Path::new(&filename)
        .file_name().and_then(|n| n.to_str())
        .ok_or_else(|| runesh_core::AppError::BadRequest("Invalid filename".into()))?;
    let path = std::path::Path::new("./uploads").join(safe_name);
    if !path.exists() { return Err(runesh_core::AppError::NotFound("File not found".into())); }
    let data = tokio::fs::read(&path).await.map_err(|e| runesh_core::AppError::Internal(e.to_string()))?;
    let ct = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    Ok(axum::response::Response::builder()
        .header("Content-Type", ct)
        .header("X-Content-Type-Options", "nosniff")
        .body(axum::body::Body::from(data)).unwrap())
}
"#);
    }

    format!(r#"use std::net::SocketAddr;

use axum::{{routing::{{get, post}}, Router, Json, middleware}};
use clap::Parser;
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

use runesh_core::shutdown_signal;
{extra_imports}
/// {name} API server
#[derive(Parser)]
#[command(name = "{name}", version, about = "{name} API server")]
struct Cli {{
    /// Port to listen on
    #[arg(short, long, env = "PORT", default_value = "{port}")]
    port: u16,

    /// Host to bind to
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,

    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,

    /// CORS allowed origins (comma-separated, or * for all)
    #[arg(long, env = "CORS_ORIGINS", default_value = "*")]
    cors_origins: String,

    /// Run database migrations on startup
    #[arg(long, env = "RUN_MIGRATIONS", default_value = "true")]
    migrate: bool,
{extra_cli_fields}}}

#[tokio::main]
async fn main() {{
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&cli.log_level))
        )
        .init();

    let pool = runesh_core::db::create_pool(Some(&cli.database_url))
        .await
        .expect("Failed to connect to database");

    if cli.migrate {{
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");
        tracing::info!("Migrations applied");
    }}

    let cors_origins: Vec<&str> = cli.cors_origins.split(',').map(|s| s.trim()).collect();

    tokio::fs::create_dir_all("./uploads").await.ok();

    let app = Router::new()
        .route("/api/v1/health", get(health))
{extra_routes}{extra_middleware}        .layer(runesh_core::middleware::cors::cors_layer(&cors_origins))
        .layer(middleware::from_fn(runesh_core::middleware::security_headers::security_headers_middleware))
        .layer(middleware::from_fn(runesh_core::middleware::logging::logging_middleware))
        .layer(middleware::from_fn(runesh_core::middleware::request_id::request_id_middleware))
        .with_state(pool);
{extra_setup}
    let addr: SocketAddr = format!("{{}}:{{}}", cli.host, cli.port)
        .parse()
        .expect("Invalid host:port");
    tracing::info!("Listening on {{addr}}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}}
{openapi_struct}
{health_annotation}async fn health(axum::extract::State(pool): axum::extract::State<PgPool>) -> Json<serde_json::Value> {{
    runesh_core::middleware::health::health_handler(axum::extract::State(pool)).await
}}
{extra_handlers}
"#,
        name = c.name,
        port = c.port,
        extra_imports = extra_imports,
        extra_cli_fields = extra_cli_fields,
        extra_routes = extra_routes,
        extra_middleware = extra_middleware,
        extra_setup = extra_setup,
        extra_handlers = extra_handlers,
        health_annotation = if c.with_openapi {
            "#[utoipa::path(get, path = \"/api/v1/health\", tag = \"System\", responses((status = 200, description = \"Health check\")))]\n"
        } else { "" },
        openapi_struct = if c.with_openapi {
            format!(r#"
/// OpenAPI documentation.
/// Add your routes and schemas here as you build them.
#[derive(OpenApi)]
#[openapi(
    info(title = "{name} API", version = "0.1.0"),
    paths(health),
    components(schemas(runesh_core::error::ErrorBody)),
    tags(
        (name = "System", description = "Health and system endpoints"),
    ),
)]
struct ApiDoc;
"#, name = c.name)
        } else {
            String::new()
        },
    )
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
    let mut editor_deps = String::new();
    if c.with_editor {
        editor_deps = r#",
    "novel": "^1.0.2",
    "@tiptap/extension-table": "^2.27.2",
    "@tiptap/extension-table-cell": "^2.27.2",
    "@tiptap/extension-table-header": "^2.27.2",
    "@tiptap/extension-table-row": "^2.27.2",
    "tiptap-extension-global-drag-handle": "^0.1.18",
    "lowlight": "^3.3.0",
    "use-debounce": "^10.1.0",
    "@tailwindcss/typography": "^0.5.19""#.into();
    }

    format!(r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "dev": "next dev",
    "build": "next build --webpack",
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
    "tw-animate-css": "^1.4.0"{editor_deps}
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
}}"#, name = c.name, ui_dep = c.npm_ui_dep(), editor_deps = editor_deps)
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
  transpilePackages: ["@mydrift-user/runesh-ui"],
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

pub const GLOBALS_CSS_IMPORT: &str = r#"@import "tailwindcss";
@import "tw-animate-css";
@import "shadcn/tailwind.css";
@import "@mydrift-user/runesh-ui/src/styles/globals.css";

@plugin "@tailwindcss/typography";
"#;

pub const USE_MOBILE: &str = r#"import * as React from "react"

const MOBILE_BREAKPOINT = 768

export function useIsMobile() {
  const [isMobile, setIsMobile] = React.useState<boolean | undefined>(undefined)

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
    const onChange = () => {
      setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    }
    mql.addEventListener("change", onChange)
    setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    return () => mql.removeEventListener("change", onChange)
  }, [])

  return !!isMobile
}
"#;

pub fn layout_tsx(c: &ProjectConfig, is_desktop: bool) -> String {
    let title = if is_desktop { format!("{} Desktop", c.name) } else { c.name.clone() };

    let mut extra_imports = String::new();
    let mut inner_before = String::new();
    let mut inner_after = String::new();

    if is_desktop {
        extra_imports.push_str("import { TitleBar } from \"@/components/layout/title-bar\";\n");
        inner_before.push_str(&format!("              <TitleBar title=\"{}\" />\n", title));
    }

    if !is_desktop && c.with_dashboard {
        extra_imports.push_str("import { AppShell } from \"@/components/app-shell\";\n");
        inner_before.push_str("              <AppShell>\n");
        inner_after.push_str("              </AppShell>\n");
    }

    format!(r#""use client";

import "./globals.css";
import {{ Toaster }} from "sonner";
import {{ ThemeProvider }} from "@mydrift-user/runesh-ui/src/components/providers/theme-provider";
import {{ QueryProvider }} from "@mydrift-user/runesh-ui/src/components/providers/query-provider";
import {{ AuthProvider }} from "@mydrift-user/runesh-ui/src/components/providers/auth-provider";
import {{ CHIRON_GOROUND_URL, FONT_FAMILY_SANS }} from "@mydrift-user/runesh-ui/src/fonts";
{extra_imports}
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
{inner_before}              {{children}}
{inner_after}            </AuthProvider>
          </QueryProvider>
        </ThemeProvider>
        <Toaster />
      </body>
    </html>
  );
}}
"#, title = title, extra_imports = extra_imports, inner_before = inner_before, inner_after = inner_after)
}

pub fn home_page(c: &ProjectConfig) -> String {
    format!(r#"import {{ PageHeader }} from "@/components/layout/page-header";

export default function Home() {{
  return (
    <div className="p-6">
      <PageHeader title="{name}" description="Welcome to your new project." />
    </div>
  );
}}
"#, name = c.name)
}

pub const UTILS_TS: &str = r#"export { cn } from "@mydrift-user/runesh-ui/src/lib/utils";
"#;

// ── Dashboard shell template ────────────────────────────────────────────────

pub fn app_shell(c: &ProjectConfig) -> String {
    format!(r#""use client";

import {{ usePathname }} from "next/navigation";
import {{ Home, Settings, FileText, Table2 }} from "lucide-react";
import {{ AppSidebar, type NavItem }} from "@/components/layout/app-sidebar";
import {{ DashboardShell }} from "@/components/layout/dashboard-shell";
import {{ SearchBar, type SearchResult }} from "@/components/layout/search-bar";
import {{ useAuth }} from "@mydrift-user/runesh-ui/src/components/providers/auth-provider";

const navItems: NavItem[] = [
  {{ title: "Dashboard", href: "/", icon: Home }},
  {{ title: "Editor", href: "/editor", icon: FileText }},
  {{ title: "Examples", href: "/examples", icon: Table2 }},
  {{ title: "Settings", href: "/settings", icon: Settings, adminOnly: true }},
];

async function onSearch(query: string): Promise<SearchResult[]> {{
  // Replace with your actual search API
  return navItems
    .filter((item) => item.title.toLowerCase().includes(query.toLowerCase()))
    .map((item) => ({{
      id: item.href,
      title: item.title,
      href: item.href,
      group: "Pages",
    }}));
}}

export function AppShell({{ children }}: {{ children: React.ReactNode }}) {{
  const pathname = usePathname();
  const {{ user, logout, isLoading, isAuthenticated }} = useAuth();

  // Public pages bypass the shell
  if (pathname.startsWith("/login") || pathname.startsWith("/auth")) {{
    return <>{{children}}</>;
  }}

  const sidebar = (
    <AppSidebar
      navItems={{navItems}}
      user={{user ? {{ username: user.name, email: user.email, role: user.role }} : null}}
      onLogout={{logout}}
      brandIcon={{<div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary text-primary-foreground text-sm font-bold">{{"{initial}"}}</div>}}
      brandName="{name}"
    />
  );

  return (
    <DashboardShell
      sidebar={{sidebar}}
      searchBar={{<SearchBar onSearch={{onSearch}} placeholder="Search..." />}}
      shortcutHint={{
        <kbd className="pointer-events-none hidden h-5 select-none items-center gap-1 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground sm:inline-flex">
          <span className="text-xs">Ctrl</span>K
        </kbd>
      }}
    >
      {{children}}
    </DashboardShell>
  );
}}
"#, name = c.name, initial = c.name.chars().next().unwrap_or('R').to_uppercase())
}

// ── Data table example page ─────────────────────────────────────────────────

pub fn data_table_page(_c: &ProjectConfig) -> String {
    r#""use client";

import { useState } from "react";
import { PageHeader } from "@/components/layout/page-header";
import { DataTable, type DataTableColumn } from "@/components/ui/data-table";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { formatRelativeTime, formatFileSize } from "@mydrift-user/runesh-ui/src/lib/format";
import { Button } from "@/components/ui/button";
import { Trash2 } from "lucide-react";

// Example data type
interface ExampleItem {
  id: string;
  name: string;
  email: string;
  role: string;
  size: number;
  createdAt: string;
}

// Example data
const DEMO_DATA: ExampleItem[] = Array.from({ length: 50 }, (_, i) => ({
  id: String(i + 1),
  name: `User ${i + 1}`,
  email: `user${i + 1}@example.com`,
  role: i % 5 === 0 ? "admin" : "user",
  size: Math.floor(Math.random() * 10_000_000),
  createdAt: new Date(Date.now() - Math.random() * 30 * 86400000).toISOString(),
}));

const columns: DataTableColumn<ExampleItem>[] = [
  { key: "name", header: "Name", getValue: (r) => r.name },
  { key: "email", header: "Email", getValue: (r) => r.email },
  {
    key: "role",
    header: "Role",
    getValue: (r) => r.role,
    renderCell: (r) => (
      <span className={`text-xs px-2 py-0.5 rounded-full ${r.role === "admin" ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground"}`}>
        {r.role}
      </span>
    ),
  },
  {
    key: "size",
    header: "Size",
    getValue: (r) => r.size,
    renderCell: (r) => formatFileSize(r.size),
  },
  {
    key: "createdAt",
    header: "Created",
    getValue: (r) => r.createdAt,
    renderCell: (r) => formatRelativeTime(r.createdAt),
  },
];

export default function ExamplesPage() {
  const [data, setData] = useState(DEMO_DATA);

  return (
    <div className="p-6 space-y-6">
      <PageHeader title="Examples" description="Data table, confirm dialog, and format utilities in action.">
        <Button variant="outline" onClick={() => setData(DEMO_DATA)}>
          Reset Data
        </Button>
      </PageHeader>

      <DataTable
        columns={columns}
        data={data}
        searchPlaceholder="Search users..."
        renderRowActions={(row) => (
          <ConfirmDialog
            trigger={<button className="text-muted-foreground hover:text-destructive"><Trash2 className="h-4 w-4" /></button>}
            title="Delete user?"
            description={`This will permanently delete ${row.name}. This action cannot be undone.`}
            confirmText="Delete"
            destructive
            onConfirm={() => setData((prev) => prev.filter((r) => r.id !== row.id))}
          />
        )}
      />
    </div>
  );
}
"#.into()
}

// ── Novel WYSIWYG Editor templates ──────────────────────────────────────────

pub fn editor_page(c: &ProjectConfig) -> String {
    format!(r#""use client";

import {{ useState }} from "react";
import dynamic from "next/dynamic";
import {{ PageHeader }} from "@/components/layout/page-header";

const RichEditor = dynamic(() => import("@/components/editor").then(m => ({{ default: m.RichEditor }})), {{
  ssr: false,
  loading: () => <div className="min-h-[500px] border rounded-lg animate-pulse bg-muted/20" />,
}});

export default function EditorPage() {{
  const [content, setContent] = useState<string | null>(null);

  return (
    <div className="p-6 space-y-6">
      <PageHeader title="Editor" description="Rich text editor with slash commands, tables, and more." />
      <RichEditor
        initialContent={{content}}
        onChange={{(json: any) => setContent(JSON.stringify(json))}}
      />
    </div>
  );
}}
"#)
}

pub const EDITOR_COMPONENT: &str = r#""use client";

import { useMemo, useState, useRef, useEffect } from "react";
import {
  EditorRoot,
  EditorContent,
  EditorCommand,
  EditorCommandItem,
  EditorCommandList,
  EditorCommandEmpty,
  type JSONContent,
  type EditorInstance,
  handleCommandNavigation,
} from "novel";
import { useDebouncedCallback } from "use-debounce";
import { defaultExtensions } from "@mydrift-user/runesh-ui/src/components/editor/extensions";
import { slashCommand, suggestionItems } from "@mydrift-user/runesh-ui/src/components/editor/slash-command";
import { EditorBubbleMenu } from "@mydrift-user/runesh-ui/src/components/editor/bubble-menu";
import { TableMenu } from "@mydrift-user/runesh-ui/src/components/editor/table-menu";
import { SearchHighlightExtension } from "@mydrift-user/runesh-ui/src/components/editor/search-highlight-extension";
import { CollapsibleHeadingExtension } from "@mydrift-user/runesh-ui/src/components/editor/collapsible-heading-extension";
import { FileHandlerExtension, type UploadFn } from "@mydrift-user/runesh-ui/src/components/editor/file-handler";
import { uploadFile } from "@mydrift-user/runesh-ui/src/lib/api-client";

// Upload handler -- sends files to your API and returns the URL
const onUpload: UploadFn = async (file: File) => {
  const formData = new FormData();
  formData.append("file", file);
  const result = await uploadFile("/api/uploads", formData) as { url?: string };
  return result?.url || "";
};

interface RichEditorProps {
  initialContent?: string | null;
  onChange?: (json: JSONContent) => void;
}

export function RichEditor({ initialContent, onChange }: RichEditorProps) {
  const extensions = useMemo(
    () => [
      ...defaultExtensions,
      slashCommand,
      SearchHighlightExtension,
      CollapsibleHeadingExtension,
      FileHandlerExtension.configure({ onUpload }),
    ],
    []
  );
  const [editorInstance, setEditorInstance] = useState<EditorInstance | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Listen for file uploads from slash commands
  useEffect(() => {
    if (!editorInstance) return;
    const handler = async (e: Event) => {
      const file = (e as CustomEvent).detail?.file as File;
      if (!file) return;
      const url = await onUpload(file);
      if (!url) return;

      if (file.type.startsWith("image/")) {
        editorInstance.chain().focus().insertContent({ type: "image", attrs: { src: url } }).run();
      } else if (file.type.startsWith("video/")) {
        editorInstance.chain().focus().insertContent({ type: "video", attrs: { src: url, fileName: file.name } }).run();
      } else if (file.type.startsWith("audio/")) {
        editorInstance.chain().focus().insertContent({ type: "audio", attrs: { src: url, fileName: file.name } }).run();
      } else {
        editorInstance.chain().focus().insertContent({
          type: "fileAttachment",
          attrs: { src: url, fileName: file.name, fileSize: file.size, fileType: file.type },
        }).run();
      }
    };
    document.addEventListener("editor-file-upload", handler);
    return () => document.removeEventListener("editor-file-upload", handler);
  }, [editorInstance]);

  const parsedContent = useMemo(() => {
    if (!initialContent) return undefined;
    try {
      const parsed = JSON.parse(initialContent);
      if (parsed?.type === "doc") return parsed;
    } catch {}
    return undefined;
  }, [initialContent]);

  const debouncedUpdate = useDebouncedCallback((editor: EditorInstance) => {
    onChange?.(editor.getJSON());
  }, 500);

  // Click on empty area below content focuses editor at end
  const handleWrapperClick = (e: React.MouseEvent) => {
    if (!editorInstance) return;
    const target = e.target as HTMLElement;
    if (target === scrollRef.current || target.closest('.ProseMirror') === null) {
      editorInstance.chain().focus().setTextSelection(editorInstance.state.doc.content.size).run();
    }
  };

  return (
    <div ref={scrollRef} className="relative min-h-[500px] border rounded-lg overflow-y-auto cursor-text" onClick={handleWrapperClick}>
      {editorInstance && (
        <TableMenu editor={editorInstance} scrollContainer={scrollRef.current} />
      )}
      <EditorRoot>
        <EditorContent
          extensions={extensions}
          initialContent={parsedContent}
          onUpdate={({ editor }) => {
            setEditorInstance(editor);
            debouncedUpdate(editor);
          }}
          onCreate={({ editor }) => setEditorInstance(editor)}
          editorProps={{
            handleDOMEvents: {
              keydown: (_view, event) => handleCommandNavigation(event),
            },
            attributes: {
              class:
                "prose prose-neutral dark:prose-invert prose-headings:font-bold focus:outline-none max-w-3xl mx-auto px-8 sm:px-12 py-8 pb-32",
            },
          }}
          className="min-h-[500px]"
        >
          <EditorCommand className="z-50 h-auto max-h-[330px] w-72 overflow-y-auto rounded-lg border border-muted bg-background px-1 py-2 shadow-xl">
            <EditorCommandEmpty className="px-2 text-muted-foreground text-sm">
              No results
            </EditorCommandEmpty>
            <EditorCommandList>
              {suggestionItems.map((item) => (
                <EditorCommandItem
                  value={item.title}
                  onCommand={(val) => item.command?.(val)}
                  className="flex w-full items-center space-x-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent aria-selected:bg-accent cursor-pointer"
                  key={item.title}
                >
                  <div className="flex h-10 w-10 items-center justify-center rounded-md border border-muted bg-background">
                    {item.icon}
                  </div>
                  <div>
                    <p className="font-medium">{item.title}</p>
                    <p className="text-xs text-muted-foreground">{item.description}</p>
                  </div>
                </EditorCommandItem>
              ))}
            </EditorCommandList>
          </EditorCommand>
          <EditorBubbleMenu />
        </EditorContent>
      </EditorRoot>
    </div>
  );
}
"#;

pub fn dot_env(c: &ProjectConfig) -> String {
    let mut env = format!(r#"# ── Server ─────────────────────────────────────────────────────────────────
DATABASE_URL=postgres://{db}:{db}@localhost:5432/{db}
JWT_SECRET=change-this-to-a-random-64-char-string-in-production!!
PORT={port}
RUST_LOG=info
"#, db = c.db_name, port = c.port);

    if c.with_openapi {
        env.push_str("SWAGGER_ENABLED=true\n");
    }

    if c.with_docker {
        env.push_str(&format!(r#"
# ── Docker ─────────────────────────────────────────────────────────────────
POSTGRES_PASSWORD=changeme
APP_PORT=8080
# NPM_TOKEN=ghp_xxx  # GitHub token for @mydrift-user/runesh-ui package (if private)
"#));
    }

    if c.with_auth {
        env.push_str(r#"
# ── OIDC (uncomment to enable SSO) ────────────────────────────────────────
# OIDC_ISSUER=https://login.microsoftonline.com/YOUR_TENANT_ID/v2.0
# OIDC_CLIENT_ID=your-client-id
# OIDC_CLIENT_SECRET=your-client-secret
# OIDC_REDIRECT_URI=http://localhost:8080/api/auth/callback
# OIDC_SCOPE=openid profile email offline_access
"#);
    }

    env
}

pub const GITIGNORE: &str = r#"# Rust
target/

# Cargo local overrides (generated by runesh init --local)
.cargo/

# Node
node_modules/
.next/
out/

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

# Copy package files and .npmrc (for GitHub Packages registry)
COPY web/package.json web/bun.lock* web/.npmrc* ./
# If @mydrift-user/runesh-ui is on a private GitHub Packages registry, pass a token:
#   docker build --build-arg NPM_TOKEN=ghp_xxx .
ARG NPM_TOKEN
RUN if [ -n "$NPM_TOKEN" ]; then \
      echo "//npm.pkg.github.com/:_authToken=${{NPM_TOKEN}}" >> .npmrc; \
    fi
RUN bun install

# Copy source and build
COPY web/ .
COPY web/next.config.ts ./next.config.ts
RUN bun run build

# ── Stage 2: Build Rust backend ────────────────────────────────────────────
FROM rust:1-bookworm AS rust-builder
WORKDIR /build

# Copy workspace files (Cargo.toml uses git deps, no local paths needed)
COPY Cargo.toml Cargo.lock* ./
COPY crates/ crates/

# Build dependencies first (layer cache)
RUN cargo build --release --bin {name}-server 2>/dev/null || true

# Copy everything and do the real build
COPY . .
# Exclude local .cargo overrides from Docker build
RUN rm -rf .cargo
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin {name}-server

# ── Stage 3: Runtime ───────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 nodejs npm && rm -rf /var/lib/apt/lists/*

# Install Caddy reverse proxy
RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=amd64" \
    -o /usr/local/bin/caddy && chmod +x /usr/local/bin/caddy

WORKDIR /app

# Copy Next.js standalone build
COPY --from=web-builder /build/.next/standalone ./web/
COPY --from=web-builder /build/.next/static ./web/.next/static/
RUN mkdir -p ./web/public

# Copy Rust binary and migrations
COPY --from=rust-builder /build/target/release/{name}-server ./backend
COPY migrations/ ./migrations/

# Caddy config: proxy /api and /ws to backend, everything else to Next.js
RUN printf ':8080 {{\n  handle /api/* {{\n    reverse_proxy 127.0.0.1:{port}\n  }}\n  handle /ws {{\n    reverse_proxy 127.0.0.1:{port}\n  }}\n  handle {{\n    reverse_proxy 127.0.0.1:3000\n  }}\n}}\n' > /etc/Caddyfile

# Start script
RUN printf '#!/bin/sh\nset -e\ncaddy start --config /etc/Caddyfile &\ncd /app/web && HOSTNAME=0.0.0.0 PORT=3000 node server.js &\ncd /app && ./backend &\nwait\n' > /app/start.sh && chmod +x /app/start.sh

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
    build:
      context: .
      args:
        NPM_TOKEN: ${{NPM_TOKEN:-}}
    restart: unless-stopped
    ports:
      - "${{APP_PORT:-8080}}:8080"
    environment:
      DATABASE_URL: postgres://{db}:${{POSTGRES_PASSWORD:-changeme}}@db:5432/{db}
      JWT_SECRET: ${{JWT_SECRET}}
      PORT: "{port}"
      SWAGGER_ENABLED: ${{SWAGGER_ENABLED:-false}}
      RUST_LOG: ${{RUST_LOG:-info}}
      # OIDC (uncomment to enable)
      # OIDC_ISSUER: ${{OIDC_ISSUER}}
      # OIDC_CLIENT_ID: ${{OIDC_CLIENT_ID}}
      # OIDC_CLIENT_SECRET: ${{OIDC_CLIENT_SECRET}}
      # OIDC_REDIRECT_URI: ${{OIDC_REDIRECT_URI:-http://localhost:8080/api/auth/callback}}
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
    if c.with_dashboard { features.push("Dashboard shell (sidebar + toolbar + search)"); }
    if c.with_editor { features.push("Novel WYSIWYG editor (wiki/rich text, file attachments)"); }
    if c.with_data_table { features.push("Data table (sortable, paginated, searchable)"); }
    if c.with_openapi { features.push("OpenAPI / Swagger UI"); }
    if c.with_docker { features.push("Docker deployment"); }
    if c.has_tauri { features.push("Tauri v2 desktop"); }
    if c.has_extension { features.push("Chrome extension (WXT)"); }

    // Build structure section dynamically
    let mut structure = Vec::new();
    if c.has_server {
        structure.push(format!("crates/{}-server/    # Axum API server", c.name));
        structure.push("migrations/              # PostgreSQL migrations".into());
    }
    if c.has_web {
        structure.push("web/                     # Next.js frontend".into());
    }
    if c.has_desktop_frontend {
        structure.push("desktop/                 # Desktop Next.js frontend".into());
    }
    if c.has_tauri {
        structure.push("src-tauri/               # Tauri v2 desktop app".into());
    }
    if c.has_extension {
        structure.push("extension/               # Chrome extension (WXT)".into());
    }
    if c.has_any_rust() {
        structure.push("Cargo.toml               # Rust workspace".into());
    }

    // Build dev commands dynamically
    let mut dev_cmds = Vec::new();
    if c.has_server { dev_cmds.push(format!("cargo run -p {}-server", c.name)); }
    if c.has_web { dev_cmds.push("cd web && bun dev".into()); }
    if c.has_desktop_frontend { dev_cmds.push("cd desktop && bun dev".into()); }
    if c.has_tauri { dev_cmds.push("cd src-tauri && cargo tauri dev".into()); }
    if c.has_extension { dev_cmds.push("cd extension && bun dev".into()); }

    let mut stack = Vec::new();
    if c.has_server { stack.push("Backend: Rust (Axum) + PostgreSQL (SQLx)".into()); }
    if c.has_web { stack.push("Frontend: Next.js + React + shadcn/ui + Tailwind CSS v4".into()); }
    stack.push(format!("Shared code: RUNESH ({})", match &c.source {
        super::RuneshSource::Git(url) => url.as_str(),
        super::RuneshSource::Local(path) => path.as_str(),
    }));
    stack.push("Package manager: bun".into());

    format!(r#"# {name}

## Stack
{stack}

## Features
{features}

## Structure
```
{name}/
{structure}
```

## Development
```
{dev_cmds}
```
"#,
        name = c.name,
        stack = stack.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"),
        features = features.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n"),
        structure = structure.iter().map(|s| format!("├── {s}")).collect::<Vec<_>>().join("\n"),
        dev_cmds = dev_cmds.join("\n"),
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
    "build": "next build --webpack",
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
  transpilePackages: ["@mydrift-user/runesh-ui"],
};

export default nextConfig;
"#;

pub fn desktop_home_page(c: &ProjectConfig) -> String {
    format!(r#"import {{ PageHeader }} from "@/components/layout/page-header";

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

// ── Chrome Extension templates (WXT + React) ────────────────────────────────

pub fn extension_package_json(c: &ProjectConfig) -> String {
    format!(r#"{{
  "name": "{name}-extension",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "dev": "wxt",
    "dev:firefox": "wxt --browser firefox",
    "build": "wxt build",
    "zip": "wxt zip"
  }},
  "dependencies": {{
    {ui_dep},
    "react": "19.2.3",
    "react-dom": "19.2.3",
    "clsx": "^2.1.1",
    "class-variance-authority": "^0.7.1",
    "tailwind-merge": "^3.5.0",
    "lucide-react": "^0.577.0"
  }},
  "devDependencies": {{
    "@types/chrome": "^0.0.300",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "@wxt-dev/module-react": "latest",
    "wxt": "latest",
    "typescript": "^5",
    "tailwindcss": "^4",
    "@tailwindcss/postcss": "^4",
    "autoprefixer": "^10",
    "postcss": "^8"
  }}
}}"#, name = c.name, ui_dep = c.npm_ui_dep())
}

pub fn extension_wxt_config(c: &ProjectConfig) -> String {
    format!(r#"import {{ defineConfig }} from "wxt";

export default defineConfig({{
  modules: ["@wxt-dev/module-react"],
  manifest: {{
    name: "{name}",
    description: "{name} Chrome Extension",
    permissions: ["storage"],
  }},
}});
"#, name = c.name)
}

pub const EXTENSION_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["dom", "dom.iterable", "esnext"],
    "strict": true,
    "noEmit": true,
    "esModuleInterop": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "jsx": "react-jsx"
  },
  "include": ["**/*.ts", "**/*.tsx"],
  "exclude": ["node_modules", ".output", ".wxt"]
}
"#;

pub const EXTENSION_POSTCSS: &str = r#"export default {
  plugins: {
    "@tailwindcss/postcss": {},
    autoprefixer: {},
  },
};
"#;

pub fn extension_popup_html(c: &ProjectConfig) -> String {
    format!(r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{name}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="./main.tsx"></script>
  </body>
</html>
"#, name = c.name)
}

pub const EXTENSION_POPUP_MAIN: &str = r#"import React from "react";
import ReactDOM from "react-dom/client";
import "./style.css";
import { App } from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
"#;

pub fn extension_popup_app(c: &ProjectConfig) -> String {
    format!(r#"import {{ useChromeStorage }} from "@mydrift-user/runesh-ui/src/hooks/use-chrome-storage";

export function App() {{
  const [count, setCount] = useChromeStorage("popup_count", 0);

  return (
    <div className="w-80 p-4 space-y-4">
      <h1 className="text-lg font-bold">{name}</h1>
      <p className="text-sm text-muted-foreground">Chrome Extension</p>
      <button
        className="inline-flex items-center justify-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        onClick={{() => setCount((c) => c + 1)}}
      >
        Count: {{count}}
      </button>
    </div>
  );
}}
"#, name = c.name)
}

pub const EXTENSION_POPUP_CSS: &str = r#"@import "tailwindcss";

:root {
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  --primary: oklch(0.205 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --muted-foreground: oklch(0.556 0 0);
}

@media (prefers-color-scheme: dark) {
  :root {
    --background: oklch(0.145 0 0);
    --foreground: oklch(0.985 0 0);
    --primary: oklch(0.922 0 0);
    --primary-foreground: oklch(0.205 0 0);
    --muted-foreground: oklch(0.708 0 0);
  }
}

body {
  background-color: var(--background);
  color: var(--foreground);
  font-family: system-ui, sans-serif;
}
"#;

pub const EXTENSION_BACKGROUND: &str = r#"export default defineBackground(() => {
  console.log("Background service worker started");
});
"#;

pub const DOCKERIGNORE: &str = r#"target/
**/node_modules/
**/.next/
.git/
.cargo/
.env
.env.local
*.md
"#;
