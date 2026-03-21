# ── Stage 1: Build Next.js frontend ─────────────────────────────────────────
FROM node:20-alpine AS web-builder
RUN corepack enable && corepack prepare pnpm@latest --activate
WORKDIR /build

COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

COPY web/ .
RUN pnpm build

# ── Stage 2: Build Rust backend ────────────────────────────────────────────
FROM rust:1-bookworm AS rust-builder
WORKDIR /build

# Install cargo-chef for layer caching
RUN cargo install cargo-chef

# Prepare recipe (dependency layer)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo chef prepare --recipe-path recipe.json

# Cook dependencies (cached unless Cargo.toml changes)
RUN cargo chef cook --release --recipe-path recipe.json

# Build the actual binary
COPY . .
# Set SQLX_OFFLINE=true if using sqlx compile-time checks
ENV SQLX_OFFLINE=true
RUN cargo build --release

# ── Stage 3: Runtime ───────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Install Caddy for reverse proxy
RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=amd64" -o /usr/local/bin/caddy \
    && chmod +x /usr/local/bin/caddy

WORKDIR /app

# Copy Next.js standalone output
COPY --from=web-builder /build/.next/standalone ./web/
COPY --from=web-builder /build/.next/static ./web/.next/static/
COPY --from=web-builder /build/public ./web/public/

# Copy Rust binary (adjust the binary name)
COPY --from=rust-builder /build/target/release/YOUR_BINARY ./backend

# Copy migrations
COPY migrations/ ./migrations/

# Caddyfile: reverse proxy frontend + backend
RUN cat > /etc/Caddyfile <<'EOF'
:8080 {
    handle /api/* {
        reverse_proxy localhost:3001
    }
    handle /ws {
        reverse_proxy localhost:3001
    }
    handle {
        reverse_proxy localhost:3000
    }
}
EOF

# Entrypoint: start all services
RUN cat > /app/start.sh <<'SCRIPT'
#!/bin/sh
set -e
caddy start --config /etc/Caddyfile &
cd /app/web && PORT=3000 node server.js &
cd /app && ./backend &
wait -n
SCRIPT
RUN chmod +x /app/start.sh

EXPOSE 8080
CMD ["/app/start.sh"]
