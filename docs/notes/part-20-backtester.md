# Part 20: Backtester backend

## Reuse design
A backtest runs the same `Engine::tick` as live mode. `Engine<F, C>` is generic over the feed and, new in this part, over `domain::EngineClock`; `SimClock` is the default so live code is unchanged. A run swaps in `ReplayClock` (one step per trading date, 16:00 New York, `trading_years` 1/252, `calendar_years` the real gap so weekends carry theta) and `ReplayFeed` (a lookup of the close and the 20-day rolling realised vol; the first 20 dates are warm-up and the book is resolved on the next). Replay steps are `StepKind::Overnight`: the attributor rolls the P&L day, and the engine fires corporate actions, only on that kind, and every replay step is a whole day. `daily.csv` is built from each snapshot's `pnl.portfolio.today`.

Two small engine hooks: a `RollPolicy` (an expired option is replaced by the same portfolio row resolved at that date) and `PriceFeed::prices_include_dividends` (real closes already fell on the ex-date, so the engine pays the cash and skips the second price drop). An option rolled at the close of date d is in the risk and VaR computed at that close, which is the start-of-day VaR of d + 1; it first appears in the VaR used for d + 1, not d, and earns P&L from d + 1. Real splits are back-adjusted (logged); a configured split that lands on a detected price break is left to the engine, with the series expressed in pre-split terms so the live split path runs unchanged.

## No look-ahead
The row for date d carries the headline VaR99 computed at the close of d − 1, from a window of returns that ends at d − 1 (`VarCalculator::set_window`, reset before every tick). A test changes one later close and checks the earlier row's VaR is bit-for-bit identical.

## Kupiec test
An exception is a day whose loss exceeds the start-of-day VaR99. With p = 0.01, n days and x exceptions:

LR = −2 ln[(1 − p)^(n − x) p^x] + 2 ln[(1 − x/n)^(n − x) (x/n)^x], with 0 · ln 0 = 0 at x = 0 and x = n.

Under the null LR is chi-square with one degree of freedom, so the p-value is P(χ²₁ > LR) = erfc(√(LR/2)) = 2Φ(−√LR), computed with `pricing::normal::cdf`.

## What the exceptions showed (real archive, run 20260930T124101Z-bkcz)
Replayed 2026-06-04 to 2026-09-28, **n = 80** days. Total P&L +$111.5k; drawdown $45.8k (2026-06-22 to 2026-07-20); best day +$70.7k, worst −$22.7k. There were **3 exceptions against 0.8 expected**: LR 3.59, p = 0.058, not rejected at 5 %. Two were marginal (2026-07-27: loss $7.3k against VaR $6.8k; 2026-09-04: $22.7k against $22.0k) and one was not (2026-06-05: $15.8k against $10.1k, the first replayed day). Two of the three came when the VaR was at its low end ($6.8k to $48.0k, median $18.8k), the signature of a VaR that follows a lagging 20-day vol.

**Power.** With 0.8 expected exceptions the test can hardly reject anything. For n = 80 it rejects only from 4 exceptions; a model whose true exception rate is 3 % is caught 22 % of the time, and one at 5 % only 57 %. So the pass says nothing, and a longer upload is needed for a real back-test. Daily steps also make the attribution residual larger than live (a whole-day Taylor expansion): 7.6 % of the sum of |actual| here.

## Interviewer questions
1. *Why is the exception test against the previous close's VaR?* A VaR computed at today's close has seen today's move, so comparing it with today's loss is look-ahead. The start-of-day figure is what a desk holds while the day is traded.
2. *Why does 80 days tell you little at 99 %?* Under a correct model only 0.8 exceptions are expected, so one exception more or less moves LR from 1.3 (two) to 3.6 (three) to 6.6 (four), and the test rejects only from four.
3. *How does the same engine run a backtest?* The clock and the feed are the two things that differ, and both are behind traits; corporate actions, settlement, valuation, attribution, limits and VaR are the same code.
