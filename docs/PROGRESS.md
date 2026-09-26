# Progress

## Open issues

None yet.

## Log

### Part 01 - Workspace scaffold

- Built: Cargo workspace (resolver 3, edition 2024) with eight crates wired per CLAUDE.md, workspace lints, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`, `.gitignore`, `.editorconfig`, `.env.example`, `Makefile`, MIT `LICENSE`, `CLAUDE.md`, roadmap, README stub, core plan copy. Created `../storage/` and `../snapshots/` with a `README.txt` each.
- Toolchain: rustc 1.98.1, node 26.7.0, npm 11.19.0, git 2.55.0, docker 29.7.2.
- New dependencies (declared in `[workspace.dependencies]`, not yet used by any member): thiserror 2, serde 1, serde_json 1, chrono 0.4, chrono-tz 0.10, tracing 0.1, and dev crates approx 0.5, proptest 1, tempfile 3. `approx` is pinned to 0.5 because 0.6 is only a release candidate.
- Tests: 0 (no logic yet).
- Open issues: none.

### Part 02 - Black-Scholes-Merton pricing

- Built: `pricing` crate with `normal::{pdf, cdf}` (CDF via `erfc`), validated `BsInputs` with `with_spot`, `with_volatility` and `with_time_to_expiry`, `OptionType`, `PricingError` (one variant per rule) and `price` with the T = 0 and σ → 0 limits. Crate-private `Terms` helper computes d1, d2, √T and the discount factors once for Part 3.
- New dependencies: `libm` 0.2 (for `erfc`, which `std` lacks), and `serde` as an optional `serde` feature of `pricing`. `thiserror`, `approx` and `proptest` were already in the workspace and are now used by `pricing`.
- Tests: 39 (25 unit and property tests, 14 doc tests). Workspace total 39.
- Notes: `#[allow(dead_code)]` on `Terms::sqrt_t`, which Part 3 will read. `#![allow(clippy::excessive_precision, clippy::unreadable_literal)]` in two test modules, so mpmath reference values stay verbatim.
- Open issues: none.
