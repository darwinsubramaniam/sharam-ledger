# syntax=docker/dockerfile:1.7
#
# Builds the Dioxus 0.7 web bundle and serves it through Caddy. Caddy also
# reverse-proxies /api → gateway, so the browser sees a single origin (matches
# the dev `dx serve` proxy in sharam-ui/Dioxus.toml).
#
# Three stages:
#   1. css       — compile Tailwind v4 → assets/tailwind.css (Node)
#   2. dx-builder— bundle the wasm SPA (Rust + dx CLI)
#   3. runtime   — Caddy serving /srv with /api proxied to gateway
#
# The Google client ID is fetched at runtime from `GET /api/config` on the
# gateway, so the resulting image is deployment-agnostic — no build args.

ARG RUST_VERSION=1
ARG DEBIAN_VERSION=bookworm
ARG NODE_VERSION=22
ARG DX_VERSION=0.7.1
ARG BUILD_SHA=dev

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
ARG BUILD_SHA
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN cargo install dioxus-cli --locked --version ${DX_VERSION}

COPY . .
# Drop in the Tailwind output before bundling.
COPY --from=css /app/sharam-ui/assets/tailwind.css /app/sharam-ui/assets/tailwind.css

WORKDIR /app/sharam-ui
ENV SHARAM_BUILD_SHA=${BUILD_SHA}
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
