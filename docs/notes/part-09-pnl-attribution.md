# Part 09 - P&L attribution

## Key maths

For a position with Greeks (Δ, Γ, ν, Θ, ρ, vanna, volga) at the start of a tick, dS = S₁ − S₀, dσ = σ₁ − σ₀, dt = the step's `calendar_years`, dr = 0 (see below), and scale = quantity × multiplier, the second-order Taylor expansion of value V(S, σ, t, r) gives:

actual = Δ·dS + ½Γ·dS² + ν·dσ + Θ·dt + ρ·dr + (vanna·dS·dσ + ½·volga·dσ²) + residual

all scaled by `scale`. The bracketed term is `higher_order`: the cross and pure-vol-convexity terms the plain Greeks alone miss. `residual` is defined as whatever is left over after subtracting the six modelled buckets from the true, fully revalued `actual` — never computed by its own formula, so it cannot hide a mistake elsewhere in the decomposition.

## Why the residual grows near expiry

Gamma, vanna and volga all scale roughly like 1/√T for an option near the money, so as T → 0 the quadratic and cross terms the expansion truncates after are no longer small relative to the linear ones — the "small dS²" assumption behind a Taylor expansion breaks down exactly where the price itself is most convex. A settling option makes this discrete: its value jumps from a smooth BSM price to intrinsic in one step, which is a true kink no polynomial expansion captures, so that tick's whole time-value collapse lands in the residual by construction (see the `dσ = 0` choice in PROGRESS.md).

## Why per-tick beats one start-of-day-to-close step

The Taylor remainder is O(dS³) for a spot-only move (confirmed by `spot_only_residual_shrinks_by_about_eight_when_the_move_halves`: halving dS shrinks the residual by about 2³ = 8, since the leading missed term is (1/6)·V'''(S₀)·dS³). Splitting one day's move into many small ticks, each with its own fresh Greeks, replaces one large dS³ with a sum of N terms of size (dS/N)³ each — a factor of N² smaller in total. It also tracks the true path rather than the net move: a spot that runs up and back down in a day has real realised gamma P&L along the way that a single open-to-close chord would erase entirely.

## Design choices

- The risk-free rate has no mutation path in this codebase, so dr and the rho bucket are always zero — this matches the prompt's own words, not a gap in the implementation.
- `PnlAttributor` rebuilds its tracked-position map from that tick's `Active` rows every call, so an expired, errored or removed position simply stops updating rather than being explicitly evicted; its accumulated totals are never deleted.
- The day roll happens before attributing the `Overnight` step itself, so that step's own theta (the weekend gap) lands in the freshly reset new day, not the one that just closed.

## Residual share (integration test, seed 42, one full session plus the overnight roll)

Whole portfolio: **0.02 %**. Portfolio excluding `spy-call-expiring`: **0.03 %**. Every individual position's own share is under 0.05 %, and `spy-call-expiring` (the one position that settles mid-run) was not the largest of them — driving the book per tick rather than open-to-close keeps every leg's residual small, expiring or not. All comfortably under the 5 % threshold.

## Interview questions

1. **Why define the residual as a subtraction instead of computing third-order Greeks?** The buckets are meant to explain what the desk can already name (delta, gamma, vega, theta). Adding speed and colour Greeks would shrink the residual but add complexity for terms nobody quotes; a residual that is small and shown honestly is more useful than a decomposition that pretends to be exact.
2. **Why does a stock position have zero residual but an option does not?** A stock's value is linear in spot (delta ≡ 1, every other Greek ≡ 0), so the first-order term is the whole story; an option's value is genuinely curved, so higher derivatives are non-zero and a second-order expansion is necessarily approximate.
3. **What would break if attribution ran once per day instead of once per tick?** Nothing would sum incorrectly, but the residual would be much larger (per the O(N²) argument above) and any intraday round trip in spot would vanish from delta/gamma P&L, so the breakdown would misreport where the day's money actually came from even though the total P&L stayed right.
