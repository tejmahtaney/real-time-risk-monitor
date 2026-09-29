.PHONY: fmt lint test check run snapshot history storage-usage types frontend-install frontend-dev dev

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
