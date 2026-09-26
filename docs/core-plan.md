Core scope (shippable in 2-3 weeks):

Backend (Rust)

Option Greeks calculator (Black-Scholes, accurate)
In-memory position book (store positions, track live prices)
Aggregated Greeks and risk exposure across portfolio
Simple REST API: /positions, /greeks, /risk-summary
WebSocket or polling endpoint for real-time updates
Basic alerting (hardcoded limits for demo)

Frontend (React or simple HTML/JS)

Live dashboard showing:
Current positions (symbol, qty, delta, gamma, vega)
Aggregate Greeks (total delta, vega exposure)
Risk metrics (notional, VaR if you have time)
P&L breakdown (delta/gamma/theta contributions)
Charts: Greeks over time, position allocation pie
Alerts pane: what limits are breached

Data
Load from a CSV or hardcoded sample portfolio. Fetch live prices from a free API (Alpha Vantage, Polygon.io, or simulate with random walks for demo).

What makes it portfolio-strong:

Live data: use real market data or convincing simulation
Clean code: Rust idioms, proper error handling, no panics
Architecture: separate concerns (pricing, Greeks, position management, API)
README: explain the math (Greeks formulas), how to run it, what limits mean
Repo structure: proper Cargo workspace, tests for Greeks accuracy
Docker: include a Dockerfile so it runs anywhere
Demo video or GIF: 2-minute walkthrough of the dashboard updating in real time

Tech stack:

actix-web or axum for HTTP/WebSocket server
ndarray or manual math for Greeks
serde + tokio for async data handling
Frontend: React with Vite or plain HTML/Canvas for charts
Deploy: Docker on a free tier (Railway, Render, or DigitalOcean)

Differentiators:

Calculate and display gamma correctly (most tutorials skip it)
Show P&L attribution: "today you made $2k from theta decay, lost $5k to vega"
Include a simple backtester: upload historical prices, replay the risk metrics
Handle edge cases: expired options, dividend adjustments, corporate actions (stock splits)

Elevator pitch:
"Real-time options portfolio risk monitor. I calculate Greeks on live market data, aggregate across positions, and alert on breaches. Built in Rust for low-latency calculations; React frontend shows live Greeks exposure, P&L breakdown, and risk summary."

Start with the backend + basic dashboard. Once that's solid and running live, add the backtester and polish the UI. Employers will care more about the accuracy of Greeks calc and clean architecture than a fancy frontend.
