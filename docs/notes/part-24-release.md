# Part 24: hardening, demo and release (phase A)

Phase A only. The audit and soak findings, the measured numbers, the demo and the tags come in phase B, and the sections marked *phase B* are filled in then.

## Hardening findings

- **Panics (1.1).** No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` or `unreachable!` in non-test code. The `as` casts that can truncate each already carry a comment and a narrow `#[allow]`. Direct indexing is confined to `risk::var` (the Cholesky factor, the Monte Carlo loop and `BTreeMap` lookups by their own keys) and `portfolio` (an index taken from the same vector). All are in range by construction, and each site now says why. One was a real hazard: `interpolated_quantile` indexed past the end for `p > 1`, so it now clamps `p`.
- **Error leaks (1.10).** Found and fixed. `StorageError::Io`, `Json` and `RepoRootNotFound`, `PortfolioError::Io` and `BacktestError::Internal` put absolute storage paths into the JSON error body. They are now logged in full and answered with a fixed sentence. Unit tests cover it. Only `/api/storage` shows the path.
- **Bounded buffers (1.4).** History is a ring buffer (`history_capacity`, 2,400). Corporate events keep the last 50. Alerts go through a 256-slot broadcast channel and `new_alerts` is drained every tick. The recorder channel holds 10,000 and drops with a counter, shown on `/api/health`. Request bodies are capped at 64 KB and uploads at 5 MB. Running backtests are capped at 2. **WebSocket clients had no cap.** Added `MAX_WS_CLIENTS = 64`, shared by `/ws` and `/ws/backtests/{id}`. Each client holds a `watch` receiver, so a slow client cannot queue frames.
- **Traversal (1.8).** Against a live debug server on an isolated storage folder, `..`, `%2e%2e`, `..%2F..%2Fetc%2Fpasswd`, double-encoded and absolute forms on sessions, portfolio versions, market-data archive, backtests (read, results, delete) and `/load` all returned 400 or 404 and touched nothing. The DELETE with `confirm: "../sessions"` is a 400.
- **Repo stays clean (1.7).** After about three minutes of running, plus a restart, `git status` showed only my source edits.
- **`npm audit --omit=dev` (1.3).** 0 vulnerabilities.
- **Crash test (1.6).** Started a session with `RRM_STORAGE_DIR` set to a temp folder, let it record for about 2 minutes, then `kill -9`. Every line of `events.jsonl` (24) and `history.jsonl` (240) parsed, with no torn last line. `meta.json` kept `end: null`, and `portfolios/current.json` parsed. After a restart the old session was listed with `end: null` and the History page labels that "cut short". A later graceful stop wrote `end` for the new session.
- **Not run (phase B):** the soak tests, release-mode tests, `cargo audit`, `cargo deny check`, `make ci`, Docker and benchmarks.

## Soak commands (phase B)

```
cargo test --release -p server --test soak -- --ignored --nocapture --test-threads=1
```

`max_speed_ten_minutes_twenty_clients` runs 10 minutes at a 1 ms tick with 20 clients and asserts resident memory at the end is within 25 % of its size after a 1-minute warm-up. `default_speed_never_drops_records` runs 2 minutes at the 250 ms tick and asserts zero dropped records. Both print the storage growth rate. *Phase B: record the numbers here.*

## Gaps against `docs/core-plan.md` (1.11)

I did not read `docs/ROADMAP.md`, as the prompt says, so the roadmap half of this walk is not done. Against the core plan:

- **Met:** BSM Greeks with gamma; position book; aggregation; `/positions`, `/greeks`, `/risk-summary`; WebSocket; alerts; every dashboard panel listed; charts; backtester, expiry, dividends and splits; P&L attribution; Cargo workspace; Dockerfile; README with the maths.
- **Weaker than promised:**
  1. "Fetch live prices from a free API". Prices are simulated, seeded from real daily closes. The plan allows a simulation, but the README should keep saying so.
  2. "Deploy: Docker on a free tier". Deliberately local only, by `CLAUDE.md`.
  3. The rate is hand-entered, so rho P&L is always zero and the rho attribution bucket is empty.
  4. The History page shows the session that is currently running as "cut short", because it has no `end` yet. A running session and a crashed one look the same. A fix needs a "live" flag from the engine.
  5. Exchange holidays are ignored by the simulated clock.
  6. Sessions, market-data files and portfolio versions cannot be deleted from the UI yet.
  7. Demo GIF: pending (phase B).
- Nothing new has been built for these. They are for you to decide.

## Demo (phase B)

`make demo` needs the app running on a throwaway storage folder (`RRM_STORAGE_DIR=$(mktemp -d) cargo run -p server`, then `npm --prefix frontend run dev`) and ffmpeg. It records 1,440 × 900, converts with `palettegen` and `paletteuse` at 12 fps, and writes `docs/demo.gif` and `docs/demo.mp4`. A step that cannot find its element is logged and skipped, so check the video. If the straddle does not breach a limit, raise `RRM_DEMO_QUANTITY`.

## 30-second pitch (phase B fills in the numbers)

"I built a real-time options risk monitor in Rust. It prices the book with Black-Scholes-Merton and computes Greeks, VaR three ways and a full P&L attribution on every tick. The whole tick takes **[median] µs against a 250 ms budget**. A React dashboard shows it live, and the backtester replays real closes through the same engine with a Kupiec test. Every session is saved locally and survives a `kill -9`, and I checked that. The soak ran 10 minutes with 20 clients and memory stayed flat at **[number]**."
