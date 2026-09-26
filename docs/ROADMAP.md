# Roadmap

Everything runs locally. Core parts end with Part 17, when the dashboard runs live from Docker on localhost and every session is saved.

1. Workspace scaffold. Crates, lints, CLAUDE.md, roadmap, storage and snapshot folders.
2. Black-Scholes-Merton pricing. Normal CDF, prices with dividend yield, reference tests.
3. Greeks. Delta, gamma, vega, theta, rho, vanna, volga, finite-difference tests, benchmarks.
4. Implied volatility. Safeguarded Newton solver with bisection fallback.
5. Domain types and position book. Instruments, CSV portfolio, per-position valuation, expiry settlement.
6. Risk aggregation. Per-underlying and portfolio Greeks in dollar terms, exposure, concentration.
7. Real market data. Storage foundation, Alpha Vantage daily closes archived locally, fallback files, calibration.
8. Simulator. Correlated GBM with mean-reverting log vol, market-hours clock, deterministic seed.
9. P&L attribution. Per-tick Taylor decomposition into delta, gamma, vega, theta, higher order and residual.
10. Limits and alerts. TOML limits, utilisation, warning and breach states with hysteresis.
11. Engine and REST API. Single-owner tick loop, snapshots over a watch channel, axum routes on localhost.
12. WebSocket streaming and history. Live snapshot stream, alert events, history ring buffer.
13. Local storage and session recording. Session files, portfolio persistence and versions, storage API.
14. Frontend scaffold. Vite, React, TypeScript, Tailwind, generated types, live data hook.
15. Dashboard panels. Positions table, risk tiles with limit bars, alerts pane, controls.
16. Charts. Greeks over time, allocation donut, P&L attribution and headline.
17. Docker and local checks. Image serving API and frontend on localhost, storage mounted, `make ci`.

Stretch.

18. Value at Risk. Delta-normal, delta-gamma Monte Carlo, historical full revaluation, expected shortfall.
19. Corporate actions. Stock splits and discrete dividends.
20. Backtester backend. CSV upload, replay feed, saved runs, VaR exception test.
21. Backtester frontend. Upload, replay controls, results view, saved runs.
22. Storage browser. Past sessions, portfolio versions, backtest comparison, market data archive, disk usage.
23. README and maths documentation.
24. Hardening, demo GIF and v1.0.0 tag.

Later. Cloud deployment, kept in the prompts folder under `later/`.
