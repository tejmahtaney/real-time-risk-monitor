# Progress

## Open issues

- Part 13: `PositionSpec` and its strike and expiry types already serialise, but there is no CSV writer yet. Part 5 only loads.
- Part 19: option multipliers are `f64` and honoured everywhere, but nothing adjusts strikes or multipliers for splits yet.
- Part 19: the split detector (Part 7) only sees splits inside the 100-day fetched window and only matches ratios within 2 % of 1.5, 2, 3, 4, 5, 10, 20 or their inverses. A real one-day drop of about 33 % would be misread as a 3-for-2 split. It is logged as a warning, so it is visible.
- Part 11: the engine should build its first `MarketState` with `SeedData::to_market_state`, take the pre-step prices from `GbmSimulator::current_tick`, and move the market on with `MarketTick::apply_to` after every `PriceFeed::step`.
- Part 11: `SimClock::advance` returns a zero-length step while paused, so the engine may skip pricing on `from == to`. It also drops the real time it is given on an `Overnight` step, so the first tick after a close is not partial.
- Part 19: the simulator has one drift μ for every symbol, and the vol shocks of different symbols correlate through the price shocks (corr of W_i, W_j is ρ² times the price correlation). Neither is calibrated from data.
- Part 19: exchange holidays are ignored by `SimClock`, so a simulated session runs on days the real market is shut.
- Part 13: `Archive` only saves and reads. It has no listing, usage or delete support, and the storage browser (Part 22) will need those.

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

### Part 03 - Greeks

- Built: `pricing::greeks` returning a `Greeks` struct (price, delta, gamma, vega, theta, rho, vanna, volga, all raw per-unit) with the formula on each field's doc comment. `Terms` now also holds S·e^(−qT) and K·e^(−rT) and owns the value expression, so `price()` and `greeks().price` come from the same code and agree bit for bit. Put delta is −e^(−qT)·N(−d1), not e^(−qT)·(N(d1) − 1). Degenerate cases (T = 0, σ·√T below `MIN_TOTAL_VOL`) are documented on `greeks`. The `#[allow(dead_code)]` on `Terms::sqrt_t` from Part 2 is gone because Greeks read it.
- New dependency: `criterion` 0.7 as a dev-dependency of `pricing`, with default features off (no plotting or HTML reports). Reason: the benchmark in `benches/greeks.rs`.
- Tests: 53 in the workspace (38 unit and property tests, 15 doc tests). 13 new unit and property tests cover mpmath references (Hull, dividend case, Haug), T = 0, tiny σ·√T, edge cases, finite differences for calls and puts, call-put identities, signs and ranges, and exact price agreement. The property tests also passed at 300,000 cases per test.
- Benchmark (release, this machine): 40.8 ns for one `greeks()` call, 441 µs for 10,000 varied options, so 44.1 ns per option (22.7 million options a second).
- Deviations from the prompt, both in the test tolerance only. (1) Each finite-difference tolerance adds the round-off the difference cannot avoid, 8·ε·(S + K)/hⁿ, because a deep in-the-money price is made of terms of size S + K and the fixed 1e-8 floor is smaller than that noise for short-dated, low-vol options. (2) Volga uses one Richardson step, (4·D(h/2) − D(h))/3 with h = 1e-4. A plain h = 1e-4 second difference fails 1e-5 relative error at 5 % vol six deviations out of the money, where the h² truncation term alone is about 6e-5. The analytic value was right.
- Open issues: none.

### Part 04 - Implied volatility solver

- Built: `pricing::implied_volatility` taking an `IvRequest` and returning an `IvSolution` (volatility and iteration count) or an `IvError` (`Expired`, `BelowIntrinsic`, `AboveUpperBound`, `NoConvergence`, `InvalidInput`). Safeguarded Newton on vega with a bracket [1e-6, 10] tightened on every evaluation, bisection when vega is under 1e-12 or the step leaves the bracket, and Brenner-Subrahmanyam as the start guess. Validation reuses `BsInputs::new` with a placeholder σ of 1, and a non-finite price is `PricingError::NonFinite`. One `greeks()` call per iteration gives price and vega.
- New dependencies: none. A second criterion bench, `benches/implied_vol.rs`.
- Tests: 67 in the workspace (51 unit and property tests, 16 doc tests). 13 new unit and property tests, 1 new doc test. Every `IvError` variant has a test. The property tests also passed at 300,000 cases per test.
- Iterations: over a grid of 5,040 round-trip cases the mean is 8.64 and the maximum 33. Pure bisection needs about 50. The 64 limit holds over 300,000 random cases.
- Benchmark (release, this machine): 160 ns for one near-the-money solve and 762 ns for a deep out-of-the-money one.
- Choices the prompt left open. (1) The upper-bound tolerance is one-sided the same way as the lower: a price within 1e-12 × max(S, K) of S·e^(−qT) or K·e^(−rT) counts as on the bound and is `AboveUpperBound`. (2) A root below 1e-6, such as a price exactly on intrinsic, returns 1e-6 and not an error. (3) A root above 10 is `NoConvergence`, detected when the bracket collapses onto the ceiling with the price still out of reach. (4) Where vega is tiny the solver returns any σ that reproduces the price to 1e-12 × max(1, price), which need not be the σ that made it.
- Open issues: none.

### Part 05 - Domain types and position book

- Built: `domain` with `Symbol`, `PositionId`, `Instrument`/`OptionContract` (multiplier default 100, `expiry_datetime()` at 16:00 America/New_York as UTC), `Position`, validated `UnderlyingState` and `MarketState`, the `Clock` trait with `FixedClock`, `year_fraction` (ACT/365) and `market_date`. `pricing::OptionType` is re-exported and the `serde` feature of `pricing` is on. `portfolio` with CSV parsing into `PositionSpec` (errors name line and column), relative strikes (`ATM`, `ATM+5%`) and expiries (`+0d`, `+Nd`), `resolve`, `PositionBook` (`add`, `remove`, `get`, `iter`, `underlyings`), `value_all` giving `PositionRisk` with reporting Greeks and a status, and `settle_expired` for cash settlement. `data/sample_portfolio.csv` has 12 positions over SPY, AAPL, MSFT, NVDA and TSLA.
- New dependency: `csv` 1, for the portfolio CSV parser. `portfolio` also depends on `pricing` directly, for `greeks` and `BsInputs` (no cycle).
- Tests: 168 in the workspace (up from 67). `domain` has 31 unit and 10 doc tests. `portfolio` has 54 unit, 3 integration and 3 doc tests.
- Choices the prompt left open. (1) Strike rounding picks the step from the raw strike: below 100 rounds to 1, 100 up to but not including 1,000 rounds to 5, 1,000 and above rounds to 10. (2) Relative expiries count from the New York date of the start time, and `+0d` is exactly that date. (3) `csv` comment lines (`#`) are allowed, and lines are counted physically for error messages. (4) A settled position has market value 0 because the cash was paid out, and its P&L is all in realised. (5) An option past expiry that is not settled is an `Error` row. (6) On an `Error` row the derived numbers are zero, never NaN. (7) Strike and expiry must be blank for equity and present for options.
- Open issues: three, listed under Open issues above.


### Part 06 - Risk aggregation

- Built: `risk::aggregate(&[PositionRisk]) -> RiskSummary`, a pure function. Types `UnderlyingRisk`, `PortfolioTotals` (dollar measures only, no share delta or share gamma field, and the doc comment says why), `Concentration` and `RiskSummary`, all `Serialize` and `Deserialize` with `snake_case` fields. Gross exposure is Σ |dollar delta| and contract notional is Σ |quantity × multiplier × spot|. Totals are folded from the per-underlying figures. Expired positions add to the expired count and to realised P&L, error positions add to the error count only, and neither adds to risk, exposure, market value or unrealised P&L.
- New dependencies: none new to the workspace. `risk` now depends directly on `domain` (for `Symbol`, which `portfolio` does not re-export; no cycle) and `serde`, with `chrono`, `pricing`, `serde_json` as dev-dependencies for the tests.
- Tests: 183 in the workspace (up from 168). `risk` has 12 unit and property tests, 2 integration tests and 1 doc test. The property tests also passed at 100,000 cases per test.
- Choices the prompt left open. (1) `position_count` counts active positions only. (2) An underlying with only expired positions still has an entry, with zero risk, its realised P&L and the settlement spot. An underlying with only error positions has none. (3) Concentration lists every underlying, including ones with zero gross exposure (fraction 0), largest first and ties in symbol order. It is empty when total gross exposure is zero or not finite. (4) The Part 5 sample book holds 300 shares and three short calls, so the "70 shares" covered call is a hand-built 100 shares and one call of delta 0.30. The sample book is checked separately by an integration test against 300 − 300 × delta.
- Open issues: none.


### Part 07 - Real market data and calibration

- Built: `storage` foundation: `StorageRoot` (`resolve()` is a thin wrapper over the pure `resolve_from(env, cwd)`, `at()` for tests, `join` and `subdir` that reject empty, `.`, `..`, separator, NUL and absolute parts), `write_json_atomic` (temp file, sync, rename, temp removed on failure), `write_json_new` (`<UTC>-<stem>.json` with `create_new`, `_02`, `_03` on a same-second clash), `list`, `read_json`. `marketdata`: `MarketConfig` (validated TOML), `HistorySource` trait and `AlphaVantageSource` (key injected, `Note`, `Error Message` and limit-style `Information` bodies mapped to `RateLimited` or `InvalidSymbol`, key hidden from `Debug` and from errors), `Archive`, fallback CSV reader and writer, `HistoryLoader` (fresh archive, API, any-age archive, fallback; `offline` skips the API), `calibrate` and `SeedData::to_market_state`. `config/market.toml` with cited rate and yields. `data/fallback/{SPY,AAPL,MSFT,NVDA,TSLA}.csv` with 100 real closes each (2026-05-05 to 2026-09-25). Two examples: `fetch_fallback` (writes the CSVs through the archive path) and `show_seed` (prints the calibration below).
- New dependencies: `tokio` (rt, macros, time, sync; async client and tests), `reqwest` 0.13 with rustls and no default features, plus `json` and `query` (the Alpha Vantage client), `toml` 1 (config). Dev only: `wiremock` 0.6 (client tests), `dotenvy` 0.15 (the fetch example reads `.env`), `tracing-subscriber` 0.3 with `fmt`, `env-filter` and `ansi` (log output in the examples). `storage` gained `serde`, `serde_json`, `chrono` and `thiserror`; `marketdata` gained `csv`, `serde`, `serde_json`, `chrono`, `thiserror`, `tracing`.
- Tests: 249 in the workspace (up from 183). `storage` has 15 unit and 1 doc test. `marketdata` has 49 unit tests and 1 integration test (`tests/committed_data.rs`, which loads the committed fallback files and config and calibrates them).
- Step 1 check of the Alpha Vantage docs: `TIME_SERIES_DAILY` is raw as-traded (not split or dividend adjusted), `compact` returns the latest 100 points on a free key, `full` is premium, free tier is 25 requests a day. All as planned. The documentation pages state no per-minute limit, but the live API rejected bursts with "1 request per second" (an `Information`-style limit message inside an HTTP 200). Requests are now spaced by 2 s measured from the end of the previous request (`DEFAULT_REQUEST_INTERVAL`).
- Choices the prompt left open. (1) The `Information` body counts as `RateLimited` when it mentions request limits, else it is `Information`. (2) `HistorySource` has a second method, `download`, returning bars plus raw body and parameters, because the archive stores the raw body. `daily_closes` is a default method over it. (3) The counter for a same-second clash goes after the stem (`_02`), because any other position sorts before the plain name. (4) Split ratios 1.5 and 2/3 were added to the prompt's list at your request. (5) The EWMA starts from the first squared return in the window and uses the same window as realised vol. (6) Per-symbol vols use each symbol's own closes, the correlation uses the inner join. (7) Yields are used as the continuous yield directly. (8) SPY 0.98 % and NVDA 0.44 % are StockAnalysis figures from 2026-09-25. MSFT 0.76 % ($3.92 a year) and AAPL 0.32 % ($1.08 a year) are StockAnalysis too, and are the latest payment annualised, which is close to but not strictly trailing. My first NVDA guess of 0.02 % was arithmetic, not a source, so it was dropped.
- Real run: SPY and AAPL fetched first, MSFT was rejected for breaking the one-a-second burst limit, then MSFT, NVDA and TSLA were fetched after the pacing fix. Five archive files are in `../storage/market-data/`. The API key appears in no archive file and in no tracked or untracked repo file.
- Calibrated seed (as of 2026-09-25, rate 0.041565, window 60 returns, chosen vol realised; the values were also recomputed independently in Python and match):

| Symbol | Spot | Realised vol | EWMA vol | Dividend yield |
|---|---|---|---|---|
| SPY | 771.35 | 11.18 % | 10.57 % | 0.98 % |
| AAPL | 341.07 | 28.44 % | 25.08 % | 0.32 % |
| MSFT | 516.17 | 38.59 % | 30.05 % | 0.76 % |
| NVDA | 225.07 | 39.09 % | 34.38 % | 0.44 % |
| TSLA | 372.11 | 54.71 % | 44.98 % | 0.00 % |

| Correlation | SPY | AAPL | MSFT | NVDA | TSLA |
|---|---|---|---|---|---|
| SPY | 1.000 | 0.043 | 0.507 | 0.559 | 0.602 |
| AAPL | 0.043 | 1.000 | -0.025 | -0.236 | -0.029 |
| MSFT | 0.507 | -0.025 | 1.000 | 0.057 | 0.238 |
| NVDA | 0.559 | -0.236 | 0.057 | 1.000 | 0.317 |
| TSLA | 0.602 | -0.029 | 0.238 | 0.317 | 1.000 |

  AAPL's near-zero correlation with SPY over these 60 days is unusually low. The maths was checked, so it is what the downloaded closes say, but it is worth a look. No splits were detected in any series (largest one-day move 15.7 %, TSLA).
- Open issues: three added under Open issues above (Parts 8, 13 and 19).


### Part 08 - Price simulator and market clock

- Built: in `domain`, `SimClock` (implements `Clock`; 09:30 to 16:00 New York on weekdays, holidays ignored, `advance`, `set_speed`, `pause`, `resume`), `ClockStep` (`from`, `to`, `trading_years`, `calendar_years`, `session_date`, `kind`), `StepKind` (`Intraday`, `Close`, `Overnight`), `next_session_open_after`, `TRADING_SECONDS_PER_YEAR`, and `MarketState::update` (all-or-nothing bulk update of spot and vol, which closes the Part 8 open issue). In `marketdata`, `SimulationConfig` and `VolProcessConfig` (validated TOML), the `PriceFeed` trait with `MarketTick`, `Quote` and `FeedError`, and `GbmSimulator`: exact log-step correlated GBM, exact Ornstein-Uhlenbeck step for log vol with leverage correlation, Cholesky from `nalgebra` with shrinkage towards the identity in steps of 0.05, `ChaCha8Rng` from the config seed. `config/simulation.toml` with the prompt's defaults. Example `simulate`.
- New dependencies (all in `marketdata`): `nalgebra` 0.35 (Cholesky and the shock matrix product; default features off, `std` on), `rand` 0.10 (`SeedableRng`, `RngExt`; default features off), `rand_chacha` 0.10 (`ChaCha8Rng`, as the prompt specifies), `rand_distr` 0.6 (`StandardNormal`, so there is no hand-written Box-Muller; default features off). `rand` 0.9 stays in the lockfile through `proptest`.
- Tests: 293 in the workspace (up from 249). `domain` has 49 unit and 13 doc tests (was 31 and 10). `marketdata` has 70 unit tests (was 49) and 3 integration tests (was 1). The statistical tests use fixed seeds, so they are deterministic. Two mutations were tried by hand and each failed a test: dropping the −½σ² term, and dropping the independent part of the vol shock.
- Choices the prompt left open. (1) The start override is `start_date`, a New York date, and not an instant. It opens at 09:30 that day, or on the next weekday. (2) Simulated time is rounded to whole milliseconds per step, so `to`, `trading_years` and `calendar_years` agree exactly. (3) A step with no trading time (overnight, paused) draws no random numbers, so a path does not depend on pauses. (4) A paused clock returns a zero-length `Intraday` step, also after a close. (5) An overnight step ignores the real time it is given. (6) `ClockStep.session_date` is the session the step belongs to, which for `Overnight` is the one being opened. (7) `SimClock::new` snaps a start outside a session to the next open, so the clock always starts in a session. (8) The tests check the −½σ²h correction at about 11 standard errors (σ = 0.5, h = 0.01), because with σ = 0.3 and h = 0.001 it is only 2 and a bug would pass a four-standard-error test. (9) A failed step leaves prices unchanged but a `NonFinite` failure has already drawn its random numbers, so the stream has moved on.
- Example, `cargo run -p marketdata --example simulate -- 3` (seed 42, from the committed fallback data, first session Monday 2026-09-28):

```
2026-09-28
symbol      open     close      high       low   change   vol range
AAPL      341.07    339.21    342.55    336.47   -0.54%   27.9% - 29.2%
MSFT      516.17    519.85    524.90    511.08   +0.71%   37.6% - 40.0%
NVDA      225.07    230.28    232.09    222.25   +2.31%   35.0% - 39.9%
SPY       771.35    772.50    773.92    767.28   +0.15%   10.9% - 12.1%
TSLA      372.11    366.85    377.67    365.13   -1.41%   53.8% - 57.8%

2026-09-30
symbol      open     close      high       low   change   vol range
AAPL      339.02    333.09    339.28    332.79   -1.75%   25.7% - 27.3%
SPY       778.53    772.06    778.59    768.02   -0.83%   11.9% - 13.8%
TSLA      379.81    359.84    382.09    357.74   -5.26%   54.3% - 61.1%
```

  Each day opens at the previous close, since the overnight step changes nothing. Daily moves are about σ/√252 (SPY 0.7 %, TSLA 3.4 %), and the vol ranges stay close to the seed vols.
- Open issues: four added under Open issues above (Parts 11, 11, 19, 19). The Part 8 `MarketState` item is closed.
