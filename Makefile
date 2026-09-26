.PHONY: fmt lint test check run snapshot history

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
