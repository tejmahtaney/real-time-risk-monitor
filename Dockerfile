# Multi-stage build for the `risk-monitor` binary and the frontend it serves.

# --- Stage 1: frontend -------------------------------------------------------------
FROM node:lts-alpine AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# --- Stage 2: Rust build, via cargo-chef --------------------------------------------
# cargo-chef splits "build my dependencies" from "build my own code" into two layers,
# so a source-only change (the common case) reuses the dependency layer's cache
# instead of recompiling the whole dependency graph every time.
FROM rust:1-slim-bookworm AS chef
# ring/aws-lc-rs (pulled in by reqwest's rustls backend) need a C toolchain to build.
RUN apt-get update && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --bin risk-monitor
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# `.git` is not in the build context (see .dockerignore), so `crates/engine/build.rs`
# cannot read the commit hash itself here the way a local `cargo build` can. The
# compose file passes it in as the GIT_COMMIT build argument instead; setting it as an
# environment variable before `cargo build` makes `option_env!("RRM_GIT_COMMIT_BUILD_TIME")`
# in crates/engine/src/recorder.rs pick it up and compile it into the binary, the same
# compiled-in slot a local build fills from `.git` itself.
ARG GIT_COMMIT=unknown
ENV RRM_GIT_COMMIT_BUILD_TIME=${GIT_COMMIT}
RUN cargo build --release --bin risk-monitor

# --- Stage 3: runtime ----------------------------------------------------------------
FROM debian:stable-slim AS runtime
# Only ca-certificates: reqwest's rustls backend needs trusted roots for HTTPS to
# Alpha Vantage; nothing else here talks to the network.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# A non-root user whose uid/gid default to 1000 but can be set to match the host user,
# so files this container writes into the bind-mounted storage folder are owned by
# that person on the host, not by root.
ARG UID=1000
ARG GID=1000
RUN groupadd --gid "${GID}" app \
    && useradd --uid "${UID}" --gid "${GID}" --create-home --shell /usr/sbin/nologin app

WORKDIR /app
COPY --from=builder /app/target/release/risk-monitor ./risk-monitor
COPY --from=frontend /app/frontend/dist ./frontend/dist
COPY config ./config
COPY data/sample_portfolio.csv ./data/sample_portfolio.csv
COPY data/fallback ./data/fallback
RUN mkdir -p /app/storage && chown -R "${UID}:${GID}" /app

ENV RRM_STATIC_DIR=/app/frontend/dist
ENV RRM_HOST=0.0.0.0
ENV RRM_STORAGE_DIR=/app/storage

EXPOSE 8080
USER app

# No curl in this image: the binary answers its own healthcheck (see
# crates/server/src/main.rs's `run_healthcheck`), keeping the runtime stage minimal.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/app/risk-monitor", "--healthcheck"]

ENTRYPOINT ["/app/risk-monitor"]
