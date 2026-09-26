# Part 05 - Domain types and position book

## Key maths

- Time to expiry is ACT/365: seconds to expiry ÷ (365 × 86,400). Options expire at 16:00 New York time, so `expiry_datetime()` is 20:00 UTC in summer and 21:00 UTC in winter. `chrono-tz` handles the changeover.
- A position's scale is s = quantity × multiplier. Reporting Greeks: share delta = δ·s, dollar delta = δ·s·S, share gamma = Γ·s, dollar gamma 1 % = ½·Γ·(0.01·S)²·s, vega = ν/100·s, theta = Θ/365·s, rho = ρ/100·s. Equity is δ = 1 with everything else zero and a multiplier of 1.
- Mark is the model price per unit. Market value = mark·s and unrealised P&L = (mark − entry)·s.
- Cash settlement at expiry: intrinsic = max(ω·(S − K), 0) with ω = +1 for a call and −1 for a put. Realised P&L = (intrinsic − entry)·s.

## Design choices

- **Relative strikes and expiries.** A committed CSV with fixed strikes and dates goes stale: a year on, the options are deep out of the money or already expired. `ATM+5%` and `+30d` resolve against the spot and date at load time, so the demo book is always near the money with live expiries. Strikes round to a listed strike (1, 5 or 10 by price band). `+Nd` moves to the next Friday because listed options expire on Fridays. `+0d` is left as today, so the demo can show an expiry.
- **Load and resolve are separate.** `parse_csv` gives unresolved `PositionSpec`s and `resolve` needs a market and a start time, so one file can be resolved against many markets in tests. Parse errors name the line and the column.
- **Blank entry price means the theoretical value at load**, so day-one P&L starts at zero and later P&L is only the move since start.
- **Validated constructors.** `Symbol`, `UnderlyingState` and `MarketState` cannot hold NaN, and deserialising goes through the same checks. Bad data is a typed error at the boundary and not a NaN in the risk.
- **One bad position never stops the book.** `value_all` turns any failure (missing underlying, bad strike, unsettled expiry) into an `Error` row with zeros. An option past its expiry that has not been settled is an error, so stale Greeks cannot reach risk.
- **Settlement lives in the book**, not in `Position`, so `Position` stays plain data. Settled positions stay for display with zero Greeks. `settle_expired` is idempotent.
- **Cash settlement is a simplification.** Real equity options settle into shares: an exercised call buys 100 shares per contract at the strike and the seller delivers them. Here that is collapsed into its cash value at the spot the engine sees, which is the tick price and not exactly the 16:00 close.

## Interview questions

1. **Why can share delta not be summed across underlyings?** 100 shares of SPY and 100 of NVDA are different units at different prices, so the sum means nothing. Share delta tells you the hedge for one underlying. Dollar delta (share delta × spot) is in one currency and adds, which is why only dollar measures aggregate to the portfolio.
2. **Why is the multiplier an `f64`?** A 3-for-2 split leaves a contract on 150 shares (Part 19). Keeping it a float means the adjustment needs no type change.
3. **Why is `+0d` not moved to a Friday?** Its job is to have an option expire today, whatever the weekday, so the dashboard can show settlement. Every other `+Nd` is a listed weekly expiry.
