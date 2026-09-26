# Progress

## Open issues

- Part 13: `PositionSpec` and its strike and expiry types already serialise, but there is no CSV writer yet. Part 5 only loads.
- Part 8: `MarketState` has `insert` but no bulk update of spots. The simulator needs one.
- Part 19: option multipliers are `f64` and honoured everywhere, but nothing adjusts strikes or multipliers for splits yet.

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

