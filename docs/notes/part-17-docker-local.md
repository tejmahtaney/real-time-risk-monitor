# Part 17 - Docker and local checks

## The three build stages

1. **`frontend`** (`node:lts-alpine`) — `npm ci` then `npm run build`, producing `frontend/dist`: hashed `assets/*.js`/`*.css` plus a root `index.html`.
2. **`chef`/`planner`/`builder`** (`rust:1-slim-bookworm`) — `cargo-chef` splits dependency compilation from the workspace's own code. `planner` copies only `Cargo.toml`/`Cargo.lock`/`crates/` and runs `cargo chef prepare`, which reads the dependency graph without touching source bodies. `builder` copies the resulting `recipe.json` and runs `cargo chef cook --release` from *that*, before the real source is copied in at all. Only then does the actual `crates/` tree land in the image and `cargo build --release --bin risk-monitor` run.
3. **`runtime`** (`debian:stable-slim`) — just `ca-certificates`, the binary, `frontend/dist`, `config/`, and the two committed `data/` seed paths. Nothing here can compile anything; there is no Rust or Node toolchain in the final image at all.

## Why cargo-chef

Docker's layer cache keys each `RUN` on everything that changed above it. Without `cargo-chef`, `COPY crates crates` followed by `cargo build --release` means *any* source edit — even a one-line change in `crates/pricing`— invalidates the layer that reruns `cargo build`, and that rebuild recompiles the entire dependency graph (`axum`, `reqwest`, `nalgebra`, …) from scratch, because they're all part of the same compilation unit's cache key. `cargo chef prepare` extracts a `recipe.json` that captures only the *dependency* graph (from `Cargo.lock`), independent of source content; `cargo chef cook` compiles just the dependencies against that recipe, in a layer that only invalidates when `Cargo.lock`/`Cargo.toml` themselves change. Everyday source edits then only pay for the final `cargo build`'s own-code compilation, not the dependency tree underneath it.

## Image size

**162 MB** (`docker image ls risk-monitor:local`, measured outside this sandbox). Release binary + `frontend/dist` + `ca-certificates` on `debian:stable-slim`, no Rust or Node toolchain, no `target/` directory, no `node_modules/` — `debian:stable-slim` plus `ca-certificates` is most of it. Slightly over the prompt's 150 MB aim rather than comfortably under it; not worth chasing further in this part (a `-slim` Rust base only affects the builder stage, which is discarded, and switching the runtime base again would be its own separate decision), but worth knowing about if a later part starts caring about pull time.

## The storage mount

`docker-compose.yml` bind-mounts `../storage` to `/app/storage`, the same relative path `cargo run -p server`'s own `RRM_STORAGE_DIR` default already resolves to — so a session recorded by the container and one recorded by `cargo run` land in the exact same `sessions/`, `portfolios/` and `market-data/` folders, interchangeably. The container's non-root user's uid/gid default to 1000 but are build arguments (`APP_UID`/`APP_GID` in the Makefile, `UID`/`GID` in the Dockerfile — renamed at the compose/Makefile boundary because bash's own `$UID` is a readonly shell variable), so files the container writes into that bind mount are owned by me on the host, not by a uid that only exists inside the image.

## The git commit hash

`.git` is deliberately not in the build context (`.dockerignore`), so `crates/engine/build.rs` — which reads `.git/HEAD` for a local `cargo build` — finds nothing in Docker. `docker-compose.yml` passes the real commit as the `GIT_COMMIT` build argument instead (`git rev-parse HEAD`, via the Makefile); the builder stage sets it as the `RRM_GIT_COMMIT_BUILD_TIME` environment variable immediately before `cargo build`, which `option_env!("RRM_GIT_COMMIT_BUILD_TIME")` in `crates/engine/src/recorder.rs` picks up and compiles into the binary — the exact same compiled-in slot a local build fills from `.git` itself, just sourced differently.

## Why the frontend and API share one origin

`axum` now serves the built frontend directly (`ServeDir` over `RRM_STATIC_DIR`, falling back to `index.html` for any unmatched client-side route) instead of the browser talking to a separate frontend server. One origin means no CORS is needed at all between the dashboard and its own API in Docker — `cors_origins` in `config/server.toml` exists purely for local `npm run dev` against Vite's dev server, which is a genuinely different origin (`:5173` vs `:8080`). It also means the WebSocket URL, cookies (were there any) and any future same-origin security header all behave the same whether the dashboard is loaded from Vite in development or from the container in Docker — there is exactly one place requests go, not a proxy configuration that has to be kept in sync between two environments.

## Cache headers

A path under `/assets/` (Vite's hashed output directory) gets `Cache-Control: public, max-age=31536000, immutable` — its filename changes whenever its content does, so caching it forever is always safe. `index.html`, and the SPA fallback that serves it for any other unmatched path, gets `no-cache` — it is the one file whose *content* can change on a new deploy without its *URL* ever changing, so a stale cached copy would mean the dashboard never picks up new hashed asset references at all.

## Interview questions

**Q: Why does an unknown `/api/...` path get a typed 404 instead of falling through to `index.html` like every other unmatched path?**
A: The SPA fallback exists so a client-side route survives a hard refresh or a deep link — but an API client hitting a typo'd or removed endpoint needs a real HTTP 404 with the usual `{"error":{"code":"not_found",...}}` body, not a 200 response containing an HTML page it will fail to parse as JSON. A `/api/{*rest}` wildcard route, registered alongside the real `/api/...` routes, is matched only when nothing more specific already matched (axum prioritises literal segments over a wildcard regardless of registration order), so it exists purely to intercept API-shaped 404s before they ever reach the static fallback.

**Q: Why is the healthcheck a hand-rolled TCP `GET` instead of using `curl` or `wget`?**
A: Either binary would need installing into the runtime image, working against the Part 17 goal of staying under 150 MB and installing nothing beyond `ca-certificates`. `risk-monitor --healthcheck` opens a `TcpStream` to its own `127.0.0.1:<port>`, writes a bare HTTP/1.1 request line by hand, and checks the response starts with a 2xx status line — a few lines of `std::net`, no new dependency, no extra installed package.

**Q: The Rust build stage runs as `rust:1-slim-bookworm` but the runtime is `debian:stable-slim` — doesn't that risk a glibc mismatch?**
A: Only in the unsafe direction would it matter (a binary linked against a *newer* glibc than the runtime provides refuses to start), and `bookworm` was Debian's stable release at the time this was written, so both stages currently resolve to the same base. Pinning the builder to `bookworm` explicitly, rather than also tracking `stable`, means the two stages can only ever drift in the safe direction (the runtime's glibc becomes newer once Debian's stable release moves on, which is backwards-compatible) rather than silently drifting apart in a way that could break the binary.
