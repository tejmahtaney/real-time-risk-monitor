# Changelog

## [1.0.0] - 2026-10-06

The first complete version. Each line names the part and the tag that holds it (`git show part-NN`). Part names follow the notes in `docs/notes/`.

- **Part 01, `part-01`.** Cargo workspace scaffold, Makefile, CI-style checks.
- **Part 02, `part-02`.** Black-Scholes-Merton pricing with a tail-safe normal CDF.
- **Part 03, `part-03`.** Greeks, including vanna and volga, from one shared set of intermediates.
- **Part 04, `part-04`.** Implied volatility solver.
- **Part 05, `part-05`.** Domain types, positions and the position book.
- **Part 06, `part-06`.** Portfolio aggregation into dollar measures.
- **Part 07, `part-07`.** Real market data from Alpha Vantage, a local archive of every download, and calibration.
- **Part 08, `part-08`.** Correlated price simulator and the market clock.
- **Part 09, `part-09`.** P&L attribution into delta, gamma, vega, theta, rho, higher-order and residual.
- **Part 10, `part-10` and `part-10-r2`.** Limits and alerts, then warning hysteresis and retuned demo limits.
- **Part 11, `part-11`.** The engine tick loop and the REST API.
- **Part 12, `part-12`.** WebSocket streaming with back-pressure, and the history buffer.
- **Part 13, `part-13`.** Local storage and session recording.
- **Part 14, `part-14`.** Frontend scaffold and the live data hook.
- **Part 15, `part-15`.** Dashboard panels.
- **Part 16, `part-16`.** Charts and the P&L headline.
- **Part 17, `part-17`.** Docker and the local checks (`make ci`).
- **Part 18, `part-18`.** Value at Risk: delta-normal, delta-gamma Monte Carlo and historical, with expected shortfall.
- **Part 19, `part-19`.** Corporate actions (splits) and discrete dividends.
- **Part 20, `part-20`.** Backtester backend with the Kupiec VaR test.
- **Part 21, `part-21`.** Backtester frontend.
- **Part 22, `part-22` and `part-22-r2`.** Storage browser pages, then layout and chart fixes.
- **Part 23, `part-23`.** README, architecture document and the maths write-up.
- **Part 24, `part-24` and `v1.0.0`.** Hardening, demo recording and release.
  - API error messages no longer carry file system paths. The full error is logged instead.
  - At most 64 WebSocket clients at once. The 65th gets a 429.
  - Out-of-range quantile index guarded in the VaR code, and indexing invariants documented.
  - History badges the session the engine is recording as "running". "Cut short" is kept for sessions that ended without an end time. `GET /api/sessions` and `/api/sessions/{id}` gained a `running` field.
  - `cargo deny` passes: every workspace crate is `publish = false`, the unused OpenSSL licence is gone and yanked `yoke-derive` is updated to 0.8.4.
  - The live-engine-while-backtesting test sizes itself, so it holds in debug and release builds.
  - Ignored soak tests (`crates/server/tests/soak.rs`), `deny.toml`, `scripts/measure-compute.sh` and the demo recorder (`make demo`, a GIF at 720 px and an MP4 at 1,440 px).
  - README: measured performance (whole tick median 468 µs, VaR throttled to every second tick), soak results, storage growth of 10.2 MiB per hour, and the rate described as hand-entered.
  - Known issue: a backtest replay step takes about 29 ms because the VaR window change redraws the Monte Carlo scenarios.
