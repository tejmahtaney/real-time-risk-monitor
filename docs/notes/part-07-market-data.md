# Part 07 - Real market data and calibration

## Key maths

- Log return rₜ = ln(Cₜ / Cₜ₋₁). Realised vol = sample standard deviation (n − 1) of the last 60 returns × √252.
- EWMA vol (RiskMetrics): σ²ₜ = λσ²ₜ₋₁ + (1 − λ)r²ₜ₋₁ with λ = 0.94, started from the first squared return, × √252. Hand check for r = [0.01, −0.02, 0.03]: 1e-4, then 1.18e-4, then 1.6492e-4 per day.
- Correlation: Pearson on returns from closes inner-joined by date across all symbols, last 60 aligned returns. Each pair is computed once and mirrored, the diagonal is set to exactly 1.
- Risk-free rate: Treasury par yields are semi-annual bond-equivalent, so r = 2·ln(1 + y/2). 4.20 % gives 4.15651 %.

## Why √252

Volatility scales with the square root of time if daily returns are independent, so σ_year = σ_day × √(trading days). Options time uses calendar days (ACT/365) but returns only exist on trading days, and there are about 252 of them a year. Using √365 would overstate vol by 20 %.

## Realised versus EWMA

Realised weights all 60 days equally, so a shock stays in for 60 days and then drops out in one step. EWMA weights recent days more (λ = 0.94 has an effective memory of about 16 days), so it reacts faster and fades smoothly. Realised is the default because it is simple to check by hand. Both are stored so Part 8 can be switched with one config line.

## Why split adjustment matters

The closes are raw as-traded. A 10-for-1 split is a return of ln 0.1 = −2.30. Over 60 days with 1.5 % daily vol that one return adds 2.30² / 59 ≈ 0.09 to the daily variance, so annual vol jumps from about 24 % to about 476 %, and correlations collapse towards zero. The detector matches a day-over-day ratio within 2 % of 1.5, 2, 3, 4, 5, 10, 20 or an inverse, divides earlier closes by the nominal ratio (not the observed one, so the split-day return keeps the real price move), and logs a warning.

## Data source chain and the archive

Per symbol: fresh archive file (younger than `cache_max_age_hours`), then the API (`auto` mode only), then the newest archive file of any age, then the committed `data/fallback/{SYMBOL}.csv`. `offline` skips the API. Every choice is logged with its file. After a rate limit, the API is not tried again for the remaining symbols.

Every download is saved as a new `market-data/{SYMBOL}/<UTC time>-daily.json` with the raw body, fetch time and request parameters, never the key and never overwritten. The free tier allows 25 requests a day and about one a second, so the app must not fetch on every start, and over time the archive becomes a local price history.

## Interview questions

1. **Why not use the adjusted-close endpoint?** It is premium, and adjusting for dividends changes the price level the options are struck against. Daily-close data is raw, so splits are detected and dividends enter through a yield.
2. **Why is the correlation built from an inner join on dates?** Two series with a missing day would pair returns from different periods, and Pearson would be computed on mismatched observations. The join keeps only days every symbol has.
3. **Why write to a temp file and rename?** A rename is atomic on one file system, so a crash leaves either the whole old file or the whole new one, never half a file.
