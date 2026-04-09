# ── Stage 1: Build Next.js frontend ─────────────────────────────────────────
FROM oven/bun:latest AS web-builder
WORKDIR /build

COPY web/package.json web/bun.lock* ./
RUN bun install

COPY web/ .
COPY web/next.config.ts ./next.config.ts
RUN bun run build

# ── Stage 2: Build Rust backend ────────────────────────────────────────────
FROM rust:1-bookworm AS rust-builder
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY .cargo/ .cargo/
RUN rm -rf .cargo/

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin YOUR_BINARY

# ── Stage 3: Runtime ───────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 nodejs tini \
    debian-keyring debian-archive-keyring apt-transport-https \
    && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg \
    && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' > /etc/apt/sources.list.d/caddy-stable.list \
    && apt-get update && apt-get install -y caddy \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=web-builder /build/.next/standalone ./web/
COPY --from=web-builder /build/.next/static ./web/.next/static/
RUN mkdir -p ./web/public
COPY --from=rust-builder /build/target/release/YOUR_BINARY ./backend
COPY migrations/ ./migrations/

# Create upload directory with proper permissions before switching user
RUN mkdir -p /app/uploads && chown -R 1000:1000 /app/uploads

RUN printf ':8080 {\n  request_body {\n    max_size 10GB\n  }\n  handle /api/* {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle /swagger-ui/* {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle /ws {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle {\n    reverse_proxy 127.0.0.1:3000\n  }\n}\n' > /etc/Caddyfile && chmod 644 /etc/Caddyfile

RUN printf '#!/bin/sh\nset -e\ncd /app/web && HOSTNAME=0.0.0.0 PORT=3000 node server.js &\ncd /app && ./backend &\nexec caddy run --config /etc/Caddyfile\n' > /app/start.sh && chmod +x /app/start.sh

# XDG config dirs for caddy (needs to write adapted config)
RUN mkdir -p /app/.config/caddy /app/.local/share/caddy

# Non-root user for security
RUN useradd -r -s /bin/false appuser && chown -R appuser:appuser /app
USER appuser

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/api/livez || exit 1

ENTRYPOINT ["tini", "--"]
CMD ["/app/start.sh"]
