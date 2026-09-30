# Part 19 - Corporate actions and discrete dividends

## Split adjustment rules

A split must leave a holding's value unchanged. With ratio r (4.0 is 4-for-1, 1.5 is 3-for-2, 0.1 is 1-for-10):

- **Stock:** quantity × r, entry price ÷ r, spot ÷ r.
- **Option, whole-number forward split** (2, 3, 4...): contracts × r, strike ÷ r, multiplier unchanged.
- **Option, any other ratio** (3-for-2, reverse): contracts unchanged, multiplier × r, strike ÷ r. A 3-for-2 leaves 150-share contracts.
- Entry price ÷ r in both option cases. Black-Scholes is homogeneous of degree one in (S, K), so price ÷ r on a position of size × r is the same value, and delta, vega, theta and dollar gamma come out unchanged too.
- Strikes are **not rounded**. The OCC rounds an adjusted strike to a listed increment. This model keeps 240 ÷ 1.5 = 160 exactly, and 200 ÷ 1.5 = 133.33....
- Pending dividends of the symbol are ÷ r as well.

## Escrowed dividend model

For a symbol on the `discrete` model, S* = S − Σ PV(Dᵢ) over dividends with an ex-date after now and on or before expiry, PV(Dᵢ) = Dᵢ·e^(−r τᵢ). The option is priced with spot S* and q = 0. S* differs from S by something independent of S, so delta, gamma and the other spot sensitivities equal those with respect to S*. Time and rate are different because S* moves: PV accretes at r, so θ = θ_BS(S*) − Δ·r·PV(D), and ρ = ρ_BS(S*) + Δ·Σ τᵢ·PV(Dᵢ). Both corrections are checked against finite differences. S* ≤ 0 is a typed error for that position only.

**Weakness.** The volatility applies only to S*, not to the dividend-inclusive stock. So the option sees less variance than the real stock has, and long-dated options on stocks with large dividends are undervalued. Known fixes, not built: the Haug-Haug-Lewis volatility-adjusted model, Bos-Vandermark's split of the dividend between spot and strike, and Beneder-Vorst.

## Why the `dividends` bucket exists

On the ex-date the price falls by D. That is a real fall in value and lands in `delta` and `actual`; the holder is paid quantity × D in cash. Without a bucket the cash has nowhere to go and a stock holder's P&L shows a loss that never happened. The cash goes into `dividends` and into `actual` together, so residual is untouched and the total is continuous. For options the ex-date drop is added back to the spot move used for delta, since S* barely moves (the escrow just empties).

## Design choices

- **Restarts.** An append-only ledger `portfolios/applied-actions.jsonl` records each applied action. A `last` start replays it into the seed, so the market and simulator start in the terms of the persisted book, and drops those ids from the calendar. A `sample` start ignores it and renames it aside. Replaying subtracts a dividend from the seed spot, not from the price the path had reached, which is close enough for a restart.
- **Load-time rule.** Each position lists `applied_actions`, the split ids already in its terms. Any book load (last, a version at startup, `LoadPortfolio`) applies every ledgered split the position lacks, in order. A position added after a split is stamped with them and not marked adjusted. A row with a blank entry price or a relative strike is created at today's market, so it is stamped and not adjusted. `adjusted` is only the "adj" badge.
- **Rebase.** The attributor is rebased to the previous tick revalued under the adjusted terms, on the market as the previous tick left it, so the overnight decay is kept and only the split artefact goes. On the split tick a position shows what the same tick shows without a split. Stock shows exactly zero.
- **Alpha Vantage.** `DIVIDENDS` is free-tier and returns declared future dividends, so an optional loader (off by default) adds them. `SPLITS` returns historical events only, so it cannot supply upcoming splits and is not used. The one live `SPLITS` request I made got the 1-per-second burst-limit reply, so its JSON shape is unverified.
- **Info events** are their own type (`CorporateActionEvent`), carried on the snapshot, not a `LimitKind`.

## Interview questions

1. *Why divide the option's entry price by the ratio?* Value at entry is quantity × multiplier × price, and both adjustment cases scale quantity × multiplier by r, so the price must fall by r for the position value to stay put.
2. *Why is theta different under the escrowed model?* S* falls as PV(D) accretes at r, so a fixed spot still loses value with time. Ignoring that leaves theta wrong by Δ·r·PV(D).
3. *How do you stop a restart applying a split twice?* The ledger says what has been applied; positions carry the ids of the splits in their terms; loads apply only the missing ones.
