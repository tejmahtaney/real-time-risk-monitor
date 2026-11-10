# Real-Time Risk Monitor

Options portfolio risk monitor. A Rust backend prices European options with Black-Scholes-Merton, computes Greeks on a live price feed, aggregates risk across the book, attributes P&L and raises limit alerts. A React dashboard shows it live. Everything the app produces is saved to a local storage folder.

The project was built in numbered parts. Only do the work you are given. If you find a problem outside that scope, report it instead of fixing it.

## Local only

- The app runs on this machine. The server binds to 127.0.0.1 by default. Only Docker overrides that, inside the container.
- No git remotes, no pushes, no cloud services. Outbound calls go only to the Alpha Vantage API.
- Runtime data lives under the storage folder, `../storage` next to the repo by default, overridden by `RRM_STORAGE_DIR`. Nothing writes runtime data inside the repo. `git status` stays clean after running the app.
- The app never deletes anything in the storage folder unless I trigger a delete action.
- Tests never touch the real storage folder. They use `tempfile` directories.

## Layout

| Path | Job |
|---|---|
| crates/pricing | Pure maths. Normal distribution, BSM price, Greeks, implied vol. No I/O, no async, no internal deps. |
| crates/domain | Shared types. Symbol, instruments, market state, clock trait, time conventions. |
| crates/storage | Local file storage. Storage root, folder layout, atomic writes, append-only logs, readers, usage. Generic over serde types. No HTTP. |
| crates/portfolio | Positions, position book, CSV loading and writing, per-position valuation and Greeks. |
| crates/marketdata | Real data fetch, local archive of every download, calibration, simulated price feed, market clock. |
| crates/risk | Aggregation, P&L attribution, limits and alerts, VaR. |
| crates/engine | Tick loop that joins feed, book and risk. Publishes immutable snapshots. Records sessions to storage. No HTTP. |
| crates/server | axum REST and WebSocket layer, config loading, binary `risk-monitor`. |
| frontend/ | React, Vite and TypeScript dashboard. |
| config/ | TOML config for market, simulation, limits, storage and server. |
| data/ | Committed seed files only. Sample portfolio and fallback price history. |
| docs/ | Roadmap, progress log, per-part notes, design docs. |

Dependency direction. `pricing` has no internal deps. `domain` uses `pricing`. `storage` uses `domain`. `portfolio` uses `domain`. `marketdata` uses `domain` and `storage`. `risk` uses `portfolio`. `engine` uses all of them. `server` uses `engine`. No cycles. Nothing below `server` knows about HTTP.

## Units and conventions

- Rates, yields and vols are decimals. 0.05 means 5 %. Rates are continuously compounded.
- Timestamps are `chrono::DateTime<Utc>`. The market time zone is America/New_York.
- File names that carry a time use UTC in the form `YYYYMMDDTHHMMSSZ`, so they sort in time order.
- Options are European. Equity options have a contract multiplier of 100 and expire at 16:00 New York time on the expiry date.
- Time to expiry is ACT/365 calendar time. Seconds to expiry divided by 365 × 86,400.
- Quantities are signed. Long is positive, short is negative.
- `pricing` returns raw per-unit sensitivities. Delta is ∂V/∂S. Gamma is ∂²V/∂S². Vega is ∂V/∂σ per 1.00 of vol. Theta is ∂V/∂t per year of calendar time passing, which equals −∂V/∂T. Rho is ∂V/∂r per 1.00 of rate. Vanna is ∂²V/∂S∂σ. Volga is ∂²V/∂σ².
- Reporting units apply from `portfolio` upwards. They cover a whole position, so they include quantity × multiplier.
  - Share delta is delta × qty × multiplier. Meaningful per underlying only.
  - Dollar delta is share delta × spot. Adds across underlyings.
  - Share gamma is gamma × qty × multiplier, the change in share delta per $1 move. Meaningful per underlying only.
  - Dollar gamma 1 % is ½ × gamma × (0.01 × spot)² × qty × multiplier, the gamma P&L of a 1 % move. Adds across underlyings.
  - Vega is raw vega ÷ 100 × qty × multiplier, in dollars per vol point.
  - Theta is raw theta ÷ 365 × qty × multiplier, in dollars per calendar day.
  - Rho is raw rho ÷ 100 × qty × multiplier, in dollars per 1 % rate move.
- Never add share delta or share gamma across different underlyings. Only dollar measures aggregate to portfolio level.
- Use `f64` for risk maths. Compare floats in tests with `approx`, never with `==`.

## Code rules

- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` or unchecked indexing outside tests. Return `Result` instead.
- Library crates define error enums with `thiserror`. Only `main` in the server binary may use `anyhow`.
- Validate inputs at crate boundaries. Bad market data or positions give typed errors, never a NaN that spreads.
- `pricing` does not allocate on the hot path. Compute d1, d2, N(·) and φ(·) once per option and reuse them.
- File writes never block the engine tick. JSON files are written to a temp file and renamed. Logs are append-only.
- Unit tests sit next to the code in `#[cfg(test)] mod tests`. Cross-crate and HTTP tests go in `tests/`.
- Log with `tracing`. No `println!` outside examples and scripts.
- Add a dependency only with a reason.
- Every public item has a doc comment. Pricing functions state their formula in the doc comment.
- British spelling in docs, comments and UI text. Identifiers follow normal Rust and TypeScript naming.

## Development history

I want to be able to go back to any earlier state of the code.

- Commit at each green checkpoint inside a part, when a sub-feature works and its tests pass, with the message `Part NN: <step>`. Small commits make `git log` a readable history.
- Never rewrite history. No `git rebase`, no `git commit --amend` on earlier commits, no `git reset --hard`, no force operations.
- Each part ends with a tag `part-NN` and a snapshot archive, as the checklist says.

## Commands

- `make check` runs the fmt check, clippy with `-D warnings` and all tests.
- `make snapshot` writes a compressed copy of the current commit to `../snapshots/`.
- `make history` shows recent commits and tags.
- `cargo run -p server` starts the backend on 127.0.0.1:8080 from Part 11 onwards.
- Frontend commands run inside `frontend/` from Part 14 onwards.

## End-of-part checklist

Run these in order at the end of every part.

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. If `frontend/` exists, run `npm run lint`, `npm run typecheck`, `npm test -- --run` and `npm run build` inside it.
5. Fix every failure. Never skip, weaken or delete a test to make it pass. Never add `#[allow]` without a comment that says why.
6. Commit everything with the message `Part NN: <short summary>`.
7. Tag the commit `part-NN`. If the tag exists because the part was re-run, use `part-NN-r2`, `part-NN-r3` and so on. Never move an existing tag.
8. Run `make snapshot`.
9. Reply with a short summary of what changed, the test count, the tag, and anything I must do by hand.
