# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-04-08

### Added
- **`runesh-telemetry` crate** — shared Sentry / GlitchTip error reporting with
  a `tracing-subscriber` layer, optional Tower/Axum middleware, and an
  `OidcVerifier`-style drop-in init. No-op when `RUNESH_SENTRY_DSN` is unset
  so the dep is safe to ship unconditionally.
- **`runesh-auth::OidcVerifier`** — JWKS-backed validation for OIDC bearer
  tokens (RS256/ES256/PS256/EdDSA). Discovers the IdP via
  `.well-known/openid-configuration`, caches keys with a 1 h TTL plus
  on-miss refresh for key rotation, maps IdP claims onto the existing
  `Claims` struct. Works against Keycloak, Azure EntraID, Auth0, Google.
- **Scaffolded telemetry opt-in** — both `runesh init` and `runesh new`
  prompt for Sentry/GlitchTip integration. Off by default even under `-y`.
  Generates `sentry.{client,server,edge}.config.ts`, `instrumentation.ts`,
  wraps `next.config.ts` with `withSentryConfig`, and in the Rust server
  wires `runesh_telemetry::init` + the Tower `SentryHttpLayer`.
- **Scaffolder now auto-installs shadcn components** — all 22 components
  required by dashboard / editor / data-table are pulled via `bunx shadcn add`
  after a pre-written `components.json`. `cmdk` is pre-installed via `bun add`
  so shadcn's internal peer-dep step doesn't fall into npm's React 19 conflict.
- **Scaffolder creates a Windows directory junction** (or unix symlink) for
  `@mydrift-user/runesh-ui` under `--local` mode, bypassing bun's EPERM on
  deep-tree copies.
- **`runesh update` handles both archive layouts** — tarballs and zips with
  the `runesh-<target>/<bin>` subdir layout now resolve correctly.

### Changed
- **All Rust deps bumped to latest stable**: `jsonwebtoken` 9→10, `rand`
  0.9→0.10, `sha2` 0.10→0.11, `hmac` 0.12→0.13, `reqwest` 0.12→0.13,
  `sysinfo` 0.35→0.38, `windows` 0.61→0.62, `notify` 7→8, `portable-pty`
  0.8→0.9, `zip` 2→8, `fuser` 0.15→0.17, `toml` 0.8→1, plus CLI tooling
  (console, dialoguer, indicatif). `self_update` pinned at 0.42.0 due to
  an upstream type-inference bug in 0.43/0.44.
- **All JS deps bumped**: Tiptap 2→3, lucide-react 0→1, TypeScript 5→6.
- **Scaffolder pins `next dev --webpack`** (and the desktop variant) so
  Next 16's Turbopack default doesn't break `transpilePackages` on the
  local `@mydrift-user/runesh-ui` package.
- **CSP, security headers, cookie layout** all reviewed — already solid,
  no changes needed.

### Security
- **[HIGH] `auth_middleware` static-file bypass was suffix-based** —
  `GET /api/admin/users.json` slipped through because `.json` was in the
  bypass list. Bypass now strictly prefix-based (`/_next/`, `/static/`,
  `/assets/`, `/favicon.ico`, …) and paths under `/api/` can NEVER be
  bypassed by the static rule.
- **[HIGH] `extract_client_ip` with `trust_proxy=true`** was taking the
  rightmost `X-Forwarded-For` entry — wrong per RFC 7239 and spoofable if
  the proxy doesn't strip inbound values. Switched to leftmost with a
  doc block explaining the required proxy config.
- **[HIGH] `save_upload` accepted any extension by default**, combined
  with `serve_upload` returning the file inline this was a stored-XSS
  vector. New `SAFE_UPLOAD_EXTENSIONS` public constant (jpg/png/gif/webp/
  pdf/txt/md/csv/json/mp3/mp4/webm/ogg/wav/zip — no html/js/svg/exe);
  scaffolder passes it. SVG removed from the magic-byte table entirely.
- **[HIGH] Scaffolded `serve_upload`** now emits
  `Content-Disposition: attachment` so even if a caller expands the
  allowlist to risky formats, the browser downloads instead of rendering.
- **[MEDIUM] `InMemoryRateLimiter` memory leak** — the `cleanup` method
  existed but nothing called it. `new()` now spawns a tokio task tied to
  `Arc::downgrade` that prunes expired keys every window; terminates
  automatically when the last limiter clone is dropped.
- **[MEDIUM] Docker base images pinned** — `oven/bun:1.3-alpine`,
  `rust:1.90-bookworm`, `debian:bookworm-20250929-slim`.
- **[MEDIUM] `POSTGRES_PASSWORD` and `JWT_SECRET`** in the generated
  compose file now use the `:?` operator — compose fails fast if unset
  instead of silently running with `changeme` / empty.
- **[MEDIUM] Generated compose binds the app to `127.0.0.1` by default**
  (override with `APP_BIND=0.0.0.0`). Expectation: reverse proxy in front.
- **[MEDIUM] Docker services get `security_opt: no-new-privileges`**;
  app additionally gets `cap_drop: [ALL]` + `cap_add: [NET_BIND_SERVICE]`.
  Redis configured with `--save '' --appendonly no`.
- **[LOW] Scaffolded `.gitignore` hardened**: added `.env.*` glob,
  `*.pem` / `*.key` / `*.crt` / `*.p12` / `*.pfx` / `credentials.json` /
  `secrets.json`, `uploads/`, `/data/`.
- **Dependency scan**: 0 vulnerabilities via `cargo audit` + `bun audit`.
  RUSTSEC-2023-0071 (`rsa` crate Marvin Attack) is present via
  `sqlx-mysql`'s unconditional inclusion in the sqlx meta-crate but the
  vulnerable codepath never runs since RUNESH only connects to Postgres.
  Suppressed in `.cargo/audit.toml` with the full justification.

### Fixed
- Scaffolder template bugs exposed during e2e testing:
  - `server_main` wrote literal `{{` / `}}` in the auth middleware chain
    (raw string values are not format!-escaped when interpolated as values).
  - `rate_limit_layer` was passed `RateLimiter` instead of the
    `RateLimiterBackend::InMemory` wrapper it now expects.
  - `health` handler return type was `Json<Value>` but `runesh_core`'s
    actual signature is `(StatusCode, Json<Value>)`.
  - `next.config.ts` used a `url` field on `withSentryConfig` that
    doesn't exist in `@sentry/nextjs` 8.
- `sentry-tracing 0.47` `HubSwitchGuard` panic across tokio worker
  threads — `runesh-telemetry::tracing_layer()` now disables span
  tracking (`.span_filter(|_| false)`); events still flow.
- `@runesh/ui` shadcn API drift — `search-bar.tsx` dropped invalid
  `title`/`description` props on `CommandDialog`; `app-sidebar.tsx` and
  `confirm-dialog.tsx` migrated from `render={...}` to `asChild +
  child-element` pattern; `editor/extensions.ts` dropped hand-typed
  prosemirror signatures that were too narrow for the actual
  `readonly Transaction[]` types.
- Pre-existing tauri 2.10 breakage (exposed by cargo update):
  `CommandExt::creation_flags` import in `process.rs`, `OsStr::new(&cow)`
  type inference in `elevation.rs`, `Menu::with_items` explicit
  `&dyn IsMenuItem<R>` coercion in `tray.rs`, `AppHandle::emit` moved to
  the new `Emitter` trait.

## [0.1.0]

Initial workspace with `runesh-cli`, `runesh-core`, `runesh-auth`,
`runesh-inventory`, `runesh-remote`, `runesh-desktop`, `runesh-vfs`,
`runesh-tun`, `runesh-tauri`, and `@runesh/ui`. Release pipeline,
self-update, installers, CI.
