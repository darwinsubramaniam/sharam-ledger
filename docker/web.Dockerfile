# syntax=docker/dockerfile:1.7
#
# Builds the Dioxus 0.7 web bundle and serves it through Caddy. Caddy also
# reverse-proxies /api → gateway, so the browser sees a single origin (matches
# the dev `dx serve` proxy in sharam-ui/Dioxus.toml).
#
# Three stages:
#   1. css       — compile Tailwind v4 → assets/tailwind.css (Node)
#   2. dx-builder— bundle the wasm SPA (Rust + dx CLI), embedding the Google
#                  client ID at compile time via sharam-ui/build.rs
#   3. runtime   — Caddy serving /srv with /api proxied to gateway
#
# The Google client ID must be supplied as a build arg or the resulting JS
# will refuse to load Google sign-in (build.rs prints a warning, but the build
# itself still succeeds).

ARG RUST_VERSION=1
ARG DEBIAN_VERSION=bookworm
ARG NODE_VERSION=22
ARG DX_VERSION=0.7.1

# ---------------------------------------------------------------------------
# Stage 1 — Tailwind CSS compile
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-alpine AS css
WORKDIR /app/sharam-ui
COPY sharam-ui/package.json sharam-ui/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY sharam-ui/tailwind.css ./tailwind.css
COPY sharam-ui/src ./src
RUN mkdir -p assets && npm run build:css

# ---------------------------------------------------------------------------
# Stage 2 — Dioxus web bundle
# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS dx-builder
ARG DX_VERSION
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN cargo install dioxus-cli --locked --version ${DX_VERSION}

COPY . .
# Drop in the Tailwind output before bundling.
COPY --from=css /app/sharam-ui/assets/tailwind.css /app/sharam-ui/assets/tailwind.css

# Forwarded into sharam-ui/build.rs so the Google client ID is embedded into
# the wasm. Build still succeeds without it (sign-in is just disabled).
ARG SHARAM_GOOGLE_CLIENT_ID=""
ENV SHARAM_GOOGLE__CLIENT_ID=${SHARAM_GOOGLE_CLIENT_ID}

WORKDIR /app/sharam-ui
RUN dx bundle --release --platform web

# ---------------------------------------------------------------------------
# Stage 3 — Caddy runtime
# ---------------------------------------------------------------------------
FROM caddy:2-alpine AS runtime
# dx 0.7 emits the static bundle under target/dx/<crate>/release/web/public.
# If you upgrade dx and the path moves, fix this COPY accordingly.
COPY --from=dx-builder /app/target/dx/sharam-ui/release/web/public /srv
COPY docker/Caddyfile /etc/caddy/Caddyfile
EXPOSE 80 443
