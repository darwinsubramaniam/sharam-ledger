# syntax=docker/dockerfile:1.7
#
# Multi-stage build for the axum gateway binary.
# Uses cargo-chef so dependency compilation is cached between source-only
# changes — typical incremental builds drop from ~10 min to ~30 s.
#
# Note on workspace scoping: `sharam-ui` is a workspace member whose default
# features pull in `dioxus/web` (wasm-only). Cooking / building the whole
# workspace for the host target would fail. We scope chef + cargo to
# `-p gateway` so only gateway's transitive deps are compiled.

ARG RUST_VERSION=1
ARG DEBIAN_VERSION=bookworm

FROM rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS chef
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build deps only, scoped to the gateway crate (cached layer).
RUN cargo chef cook --release --recipe-path recipe.json -p gateway
COPY . .
RUN cargo build --release -p gateway --bin gateway

FROM debian:${DEBIAN_VERSION}-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin sharam
WORKDIR /app
COPY --from=builder /app/target/release/gateway /usr/local/bin/gateway
USER sharam
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=5 \
    CMD curl -fsS http://localhost:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/gateway"]
