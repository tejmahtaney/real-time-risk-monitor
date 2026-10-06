.PHONY: fmt lint test check run snapshot history storage-usage types frontend-install \
	frontend-dev dev ci types-check frontend-ci docker-build docker-up docker-down hooks demo

fmt:
	cargo fmt --all

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

run:
	cargo run -p server

# Regenerates frontend/src/api/generated/ from the `#[derive(ts_rs::TS)]` Rust types
# that cross the API. TS_RS_LARGE_INT=number matches how serde_json actually encodes
# a u64 (a plain JSON number, not a string), so the generated bindings match the real
# wire format rather than ts-rs's bigint-by-default assumption.
types:
	TS_RS_EXPORT_DIR="$(CURDIR)/frontend/src/api/generated" TS_RS_LARGE_INT=number \
		cargo test --workspace export_bindings

frontend-install:
	npm --prefix frontend install

frontend-dev:
	npm --prefix frontend run dev

# Runs the backend and the Vite dev server together. `concurrently` (a frontend
# devDependency) is used rather than a hand-rolled shell job-control script, since it
# already handles prefixed, colour-coded output and killing both processes together
# on Ctrl-C (-k).
dev:
	npx --prefix frontend concurrently -k -n backend,frontend -c blue,green \
		"cargo run -p server" "npm --prefix frontend run dev"

# Archive HEAD into ../snapshots. Refuses to run on a dirty tree so every
# snapshot matches a commit.
snapshot:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "Working tree has uncommitted changes. Commit first so the snapshot matches a commit." >&2; \
		exit 1; \
	fi
	@mkdir -p ../snapshots
	@out="../snapshots/$$(date -u +%Y%m%dT%H%M%SZ)-$$(git describe --tags --always).tar.gz"; \
	git archive --format=tar.gz --prefix=risk-monitor/ -o "$$out" HEAD && echo "Wrote $$out"

history:
	git log --oneline --decorate -n 30

# Prints disk usage by top-level storage folder (market-data, portfolios, sessions, ...).
storage-usage:
	@cargo run --quiet --example storage_usage -p storage

# Everything a CI server would run for this project, in the order a change is most
# likely to fail: fast Rust checks first, then the frontend, then the (slowest)
# Docker build last, so a real bug is reported well before the costly Docker build
# even starts.
ci: check types-check frontend-ci docker-build

# Fails if `make types` regenerates bindings that differ from what is committed, so a
# Rust API change without a matching `make types` run is caught before it reaches the
# frontend as a silent drift instead of a type error.
types-check:
	$(MAKE) types
	git diff --exit-code frontend/src/api/generated

frontend-ci:
	npm --prefix frontend run lint
	npm --prefix frontend run typecheck
	npm --prefix frontend test -- --run
	npm --prefix frontend run build

# APP_UID/APP_GID default to the current host user, so files risk-monitor writes into
# the bind-mounted ../storage belong to me, not root (bash's own UID/GID are readonly
# shell variables, hence the APP_ prefix here and in docker-compose.yml). GIT_COMMIT is
# compiled into the image (see the Dockerfile) since `.git` is not in the build context.
docker-build:
	GIT_COMMIT="$$(git rev-parse HEAD)" APP_UID="$$(id -u)" APP_GID="$$(id -g)" docker compose build

docker-up:
	GIT_COMMIT="$$(git rev-parse HEAD)" APP_UID="$$(id -u)" APP_GID="$$(id -g)" docker compose up --build

docker-down:
	docker compose down

# Not run automatically — installs scripts/pre-commit (the fast fmt/clippy checks
# only) into this clone's own .git/hooks/, which nothing else here does on its own.
hooks:
	install -m 755 scripts/pre-commit .git/hooks/pre-commit
	@echo "Installed scripts/pre-commit -> .git/hooks/pre-commit"

# Records the scripted demo and converts it. Needs the app already running (`make dev`) on a
# throwaway storage folder, plus ffmpeg. The WebM is a temporary file; the outputs are
# docs/demo.gif (800 px wide, 6 fps, double speed, a 64-colour palette, no dithering, about
# 9 MB) and docs/demo.mp4 (1440 px, full speed). If the GIF is over 10 MB, lower the width
# in the scale filter below.
demo:
	@command -v ffmpeg >/dev/null || { echo "ffmpeg is required for make demo" >&2; exit 1; }
	@tmp="$$(mktemp -d)"; trap 'rm -rf "$$tmp"' EXIT; \
	RRM_DEMO_OUT="$$tmp/demo.webm" npm --prefix frontend run demo && \
	ffmpeg -y -loglevel error -i "$$tmp/demo.webm" \
		-vf "setpts=PTS/2,fps=6,scale=800:-1:flags=lanczos,palettegen=max_colors=64" "$$tmp/palette.png" && \
	ffmpeg -y -loglevel error -i "$$tmp/demo.webm" -i "$$tmp/palette.png" \
		-lavfi "setpts=PTS/2,fps=6,scale=800:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=none:diff_mode=rectangle" docs/demo.gif && \
	ffmpeg -y -loglevel error -i "$$tmp/demo.webm" -vf "scale=1440:-2,format=yuv420p" \
		-c:v libx264 -crf 23 -movflags +faststart docs/demo.mp4 && \
	ls -lh docs/demo.gif docs/demo.mp4 && \
	if [ "$$(stat -c %s docs/demo.gif)" -gt 10485760 ]; then echo "docs/demo.gif is over 10 MB; narrow the scale filter" >&2; exit 1; fi
