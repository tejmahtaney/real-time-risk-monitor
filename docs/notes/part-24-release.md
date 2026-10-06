# Part 24: hardening, demo and release

Phase A was the audit and the scripts. Phase B fixed what the controller's runs found and recorded the measured numbers. The tag and the snapshot come after the demo is re-recorded.

## Hardening findings

- **Panics (1.1).** No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` or `unreachable!` in non-test code. The `as` casts that can truncate each already carry a comment and a narrow `#[allow]`. Direct indexing is confined to `risk::var` (the Cholesky factor, the Monte Carlo loop and `BTreeMap` lookups by their own keys) and `portfolio` (an index taken from the same vector). All are in range by construction, and each site now says why. One was a real hazard: `interpolated_quantile` indexed past the end for `p > 1`, so it now clamps `p`.
- **Error leaks (1.10).** Found and fixed. `StorageError::Io`, `Json` and `RepoRootNotFound`, `PortfolioError::Io` and `BacktestError::Internal` put absolute storage paths into the JSON error body. They are now logged in full and answered with a fixed sentence. Unit tests cover it. Only `/api/storage` shows the path.
- **Bounded buffers (1.4).** History is a ring buffer (`history_capacity`, 2,400). Corporate events keep the last 50. Alerts go through a 256-slot broadcast channel and `new_alerts` is drained every tick. The recorder channel holds 10,000 and drops with a counter, shown on `/api/health`. Request bodies are capped at 64 KB and uploads at 5 MB. Running backtests are capped at 2. **WebSocket clients had no cap.** Added `MAX_WS_CLIENTS = 64`, shared by `/ws` and `/ws/backtests/{id}`. Each client holds a `watch` receiver, so a slow client cannot queue frames.
- **Traversal (1.8).** Against a live debug server on an isolated storage folder, `..`, `%2e%2e`, `..%2F..%2Fetc%2Fpasswd`, double-encoded and absolute forms on sessions, portfolio versions, market-data archive, backtests (read, results, delete) and `/load` all returned 400 or 404 and touched nothing. The DELETE with `confirm: "../sessions"` is a 400.
- **Repo stays clean (1.7).** After about three minutes of running, plus a restart, `git status` showed only my source edits.
- **`npm audit --omit=dev` (1.3).** 0 vulnerabilities.
- **Crash test (1.6).** Started a session with `RRM_STORAGE_DIR` set to a temp folder, let it record for about 2 minutes, then `kill -9`. Every line of `events.jsonl` (24) and `history.jsonl` (240) parsed, with no torn last line. `meta.json` kept `end: null`, and `portfolios/current.json` parsed. After a restart the old session was listed with `end: null` and the History page labels that "cut short". A later graceful stop wrote `end` for the new session.

## Controller findings and fixes (phase B)

- **`cargo deny`.** Advisories, licences and sources passed. Bans failed because the workspace crates count as public, so their path dependencies are wildcards. Fixed with `publish = false` on every workspace crate. The unused OpenSSL licence allowance is gone. `cargo audit` found no vulnerabilities, but `yoke-derive` 0.8.3 was yanked, so it is updated to 0.8.4.
- **Release-mode tests.** 633 passed and 1 failed. The failing test asserted a backtest takes over 300 ms, and a release build replays it in about 115 ms. The test now doubles its date count until a run lasts at least 300 ms (up to six doublings), so it needs no tuning for any build.
- **Running session.** History showed the live session as "cut short". The session list and detail now carry `running`, true when the id is the engine's current session. The badge says "running", and "cut short" is only for ended-without-end-time sessions.
- **Demo.** The straddle defaults to 15 contracts a leg, so one or two limits breach rather than eight. The recording is about 90 seconds, each pause has its panel fully in view, and the GIF is 800 px, 6 fps and double speed with a 64-colour palette (the controller measured about 8.6 MB, down from 57 MB).

## Measured numbers

All on the release Docker image with the sample book, 6 October 2026, AMD Ryzen 5 5500.

| What | Result |
|---|---|
| Whole tick (`compute_micros`, n = 596) | median 468 µs, p99 1,906 µs, max 2,195 µs, against a 250 ms budget |
| VaR, fresh computations (n = 296) | median 1,008 µs, p99 1,694 µs |

VaR's warm-up median was over its 1 ms budget, so it recomputes every second tick. The Part 12 note that the throttle never triggered no longer holds.

**Soak** (release build, the controller's cloud machine):

| Run | Result |
|---|---|
| Maximum speed, 20 WebSocket clients, 10 minutes | memory flat at 23.4 to 23.8 MB, 0 dropped records, storage 1,505 MiB per hour |
| Default speed | 0 dropped records, storage 10.2 MiB per hour |

The commands are in `crates/server/tests/soak.rs`: `cargo test --release -p server --test soak -- --ignored --nocapture --test-threads=1`.

## Gaps against `docs/core-plan.md` and `docs/ROADMAP.md` (1.11)

- **Met.** Every roadmap part from 1 to 24 has a tag and a note. Core plan: BSM Greeks with gamma; position book; aggregation; `/positions`, `/greeks`, `/risk-summary`; WebSocket; alerts; every dashboard panel listed; charts; backtester, expiry, dividends and splits; P&L attribution; Cargo workspace; Dockerfile; README with the maths.
- **Weaker than promised:**
  1. "Fetch live prices from a free API". Prices are simulated, seeded from real daily closes (roadmap Parts 7 and 8). The plan allows a simulation, and the README says so.
  2. "Deploy: Docker on a free tier", and the roadmap's "Later: cloud deployment". Deliberately local only, by `CLAUDE.md`. Not built.
  3. The rate is hand-entered, so rho P&L is always zero and the rho attribution bucket is empty (roadmap Part 9 lists no rho bucket, but the plan's Greeks include rho).
  4. Exchange holidays are ignored by the simulated clock (roadmap Part 8, "market-hours clock").
  5. Sessions, market-data files and portfolio versions cannot be deleted from the UI. Only backtest runs can (roadmap Part 22 "storage browser").
  6. A replay step takes about 29 ms because `VarCalculator::set_window` redraws the Monte Carlo scenarios. Only the Cholesky factor needs rebuilding (roadmap Part 20).
  7. The VaR throttle runs on the sample book, so VaR is a number every second tick, not every tick.
- Closed in this phase: the running session shown as "cut short".
- Nothing new has been built for the rest. They are for you to decide.

## Demo

`make demo` needs the app running on a throwaway storage folder (`RRM_STORAGE_DIR=$(mktemp -d) cargo run -p server`, then `npm --prefix frontend run dev`) and ffmpeg. It records 1,440 × 900, writes `docs/demo.mp4` at full speed and `docs/demo.gif` at 800 px, 6 fps and double speed. A step that cannot find its element is logged and skipped, so check the video. If the straddle breaches nothing, raise `RRM_DEMO_QUANTITY` above its default of 15.

## 30-second pitch

"I built a real-time options risk monitor in Rust. It prices the book with Black-Scholes-Merton and computes Greeks, VaR three ways and a full P&L attribution on every tick. The whole tick takes a median of **468 µs against a 250 ms budget**, and the worst of 596 ticks was 2.2 ms. A React dashboard shows it live, and the backtester replays real closes through the same engine with a Kupiec test. Every session is saved locally and survives a `kill -9`, and I checked that. The soak ran 10 minutes with 20 clients at maximum speed, memory stayed flat at **23.4 to 23.8 MB** and no records were dropped."
