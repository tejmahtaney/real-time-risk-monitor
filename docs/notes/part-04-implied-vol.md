# Part 04 - Implied volatility

## Key maths

The price is strictly increasing in σ, because vega = S·e^(−qT)·φ(d1)·√T > 0. So at most one σ solves BS(σ) = price, and it exists exactly when the price lies between the discounted forward intrinsic value max(ω·(S·e^(−qT) − K·e^(−rT)), 0) and the upper bound, S·e^(−qT) for a call or K·e^(−rT) for a put. Outside that range no σ works, which is why those cases are typed errors and not a search that fails.

The solver is safeguarded Newton. It starts from Brenner-Subrahmanyam, σ₀ = √(2π/T)·price/S, which is exact only at the money and is only a start point. It keeps a bracket [lo, hi] = [1e-6, 10]. Each evaluation of BS(σ) puts the root on a known side, so it moves one end of the bracket. The next σ is Newton's, σ − (BS(σ) − price)/vega, unless vega is under 1e-12 or the step leaves the bracket, when it is the midpoint. It stops at |BS(σ) − price| ≤ 1e-12 × max(1, price) or a bracket under 1e-14.

## Why plain Newton diverges, and why the bracket fixes it

Newton fits a tangent, and deep out of the money the price is almost flat in σ then rises steeply, so the tangent at a low σ is nearly horizontal. Concrete case: S = 100, K = 140, T = 0.1, r = 2 %, true σ = 30 %. The clamped start guess is 0.05, where the price is about 1e-100 and vega about 1e-96. The step (price error)/vega is then large and positive, and plain Newton lands on σ ≈ 6e92, after which vega is exactly 0 and it divides by zero. The bracket stops this in two ways. A step outside (lo, hi) is thrown away for the midpoint, and every evaluation shrinks the bracket, so the worst case is bisection, about 50 steps to 1e-14, and never a divergence.

## Design choices

- One `greeks()` call per iteration gives price and vega, so no extra evaluation. It reuses the same code path as `price()`, so a round trip is exact.
- Validation reuses `BsInputs::new` with a placeholder σ of 1. The rules cannot drift from Part 2.
- Bounds tolerance is 1e-12 × max(S, K), so a price on intrinsic through rounding does not fail. Such a root is below the 1e-6 floor and is returned as the floor.
- Where vega is tiny, many σ give the same price, so σ is not identifiable. The round-trip test requires σ to 1e-8 only where vega × 1e-8 exceeds the price tolerance, and otherwise that the recovered σ reproduces the price to 1e-10 × max(S, K).
- "Let's Be Rational" (Jäckel, 2015) is the production alternative. It is out of scope.

## Iteration counts

Over a grid of 5,040 cases across the Part 3 domain with σ from 0.02 to 2.0, calls and puts: **mean 8.64, maximum 33**. Pure bisection needs about 50. The property test found no case above 64 in 300,000 random draws. Solve time is 160 ns near the money and 762 ns deep out of the money.

## Interview questions

1. **Why bracket as well as Newton?** Newton converges quadratically near the root but is unsafe where vega is small, and vega is small exactly for deep in and out of the money options. The bracket costs one comparison per step and gives bisection's guarantee as the floor.
2. **Why can a round trip fail to recover σ?** When vega is tiny, a wide range of σ gives the same price to within tolerance. The price is recovered, the σ is not, and no algorithm can do better in double precision.
3. **Why reject a price at or above S·e^(−qT) for a call?** The call is worth at most the discounted spot, reached only as σ → ∞. A price there has no finite implied vol.
