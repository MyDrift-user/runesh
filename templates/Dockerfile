# ── Stage 1: Build Next.js frontend ─────────────────────────────────────────
FROM oven/bun:latest AS web-builder
WORKDIR /build

COPY web/package.json web/bun.lock* ./
COPY web/.npmrc* ./

# If @mydrift-user/runesh-ui is on a private GitHub Packages registry, pass a token:
#   docker build --build-arg NPM_TOKEN=ghp_xxx .
ARG NPM_TOKEN
RUN if [ -n "$NPM_TOKEN" ]; then \
      echo "//npm.pkg.github.com/:_authToken=${NPM_TOKEN}" >> .npmrc; \
    fi
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
    ca-certificates curl libssl3 nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=amd64" -o /usr/local/bin/caddy \
    && chmod +x /usr/local/bin/caddy

WORKDIR /app

COPY --from=web-builder /build/.next/standalone ./web/
COPY --from=web-builder /build/.next/static ./web/.next/static/
RUN mkdir -p ./web/public
COPY --from=rust-builder /build/target/release/YOUR_BINARY ./backend
COPY migrations/ ./migrations/

RUN printf ':8080 {\n  request_body {\n    max_size 10GB\n  }\n  handle /api/* {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle /swagger-ui/* {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle /ws {\n    reverse_proxy 127.0.0.1:3001\n  }\n  handle {\n    reverse_proxy 127.0.0.1:3000\n  }\n}\n' > /etc/Caddyfile

RUN printf '#!/bin/sh\nset -e\ncaddy start --config /etc/Caddyfile &\ncd /app/web && HOSTNAME=0.0.0.0 PORT=3000 node server.js &\ncd /app && ./backend &\nwait\n' > /app/start.sh && chmod +x /app/start.sh

EXPOSE 8080
CMD ["/app/start.sh"]
