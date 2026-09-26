# Part 06 - Risk aggregation

## Key maths

- Per position (from Part 5), with s = quantity × multiplier: share delta = δ·s, dollar delta = δ·s·S, share gamma = Γ·s, dollar gamma 1 % = ½·Γ·(0.01·S)²·s.
- Per underlying u, sum over its active positions: share delta Σδ·s, dollar delta Σδ·s·S, gross exposure Σ|δ·s·S|, contract notional Σ|s·S|.
- Portfolio: each dollar measure is Σ over underlyings. Concentration of u = gross exposure of u ÷ portfolio gross exposure, so the fractions sum to 1.
- Worked example. 100 shares plus one short call of delta 0.30 at spot 200: share delta 100 − 30 = 70, net dollar delta 14,000, gross exposure 20,000 + 6,000 = 26,000, contract notional 20,000 + 20,000 = 40,000.

## Why dollar gamma adds and share gamma does not

Share gamma is the change in share delta per $1 move of one stock. A $1 move in SPY and a $1 move in NVDA are different events of different sizes, and their share deltas are shares of different things, so 3 and 5 make no 8. Dollar gamma 1 % is the P&L of a 1 % move in that stock, ½·Γ·(0.01·S)²·s. It is in dollars for every stock, so the P&L of a 1 % move in each adds. It assumes all stocks move 1 % together, and that is a stated scenario, not a forecast. `PortfolioTotals` has no share fields, so this cannot be got wrong by accident.

## Gross exposure, net dollar delta and contract notional

- **Net dollar delta** is what you lose or gain per 1 % move of the whole book, scaled by 100. Long and short cancel, so a hedged book shows near zero.
- **Gross exposure** is Σ|dollar delta|. Long and short both count, so a hedge pair shows the size of what is held. It is the base for concentration.
- **Contract notional** is Σ|s·S|: what the contracts control if exercised, with no delta weighting. A far out-of-the-money option has tiny delta and full notional.

For a covered call the three are 14,000, 26,000 and 40,000 in the example above.

## Design choices

- **Pure function over a slice.** No state, so the engine calls it once per tick on the fresh valuations and the tests need no set-up.
- **Totals are folded from the per-underlying figures**, so the two levels cannot disagree. The property tests compare them with independent sums over positions.
- **Expired and error rows never touch risk.** Expired rows carry realised P&L into their underlying, error rows only raise a count, so a bad position is visible and never a NaN.
- **Concentration guards its divisor.** Zero or non-finite gross exposure gives an empty list, not a division by zero.
- **Sorted largest first**, so the Part 16 donut needs no sorting of its own.

## Interview questions

1. **Why is there no portfolio share delta?** Shares of different stocks are different units. Only dollar delta, which is in one currency, adds. Making the totals type omit the field makes the mistake impossible to write.
2. **Why report gross exposure when net delta exists?** Net delta hides offsetting positions. A book long 10m and short 10m has zero delta and 20m of gross exposure, and a basis or gap move hurts it. Limits often need both.
3. **Why does an option have small delta but full contract notional?** Delta is the hedge ratio, the fraction of the notional that behaves like stock today. Notional is the size of the claim. A far out-of-the-money option can jump in delta, which is what gamma measures, so notional is the outer bound of what it could come to control.
