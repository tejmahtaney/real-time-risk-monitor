# Part 02 - Black-Scholes-Merton pricing

## Key maths

For a European option with continuous dividend yield q:

- d1 = [ln(S/K) + (r − q + σ²/2)·T] / (σ·√T), d2 = d1 − σ·√T
- Call = S·e^(−qT)·N(d1) − K·e^(−rT)·N(d2)
- Put = K·e^(−rT)·N(−d2) − S·e^(−qT)·N(−d1)

**Intuition.** Under the risk-neutral measure, N(d2) is the probability the option finishes in the money. N(d1) is the same probability under the measure where the stock is the numeraire. So the call is the discounted expected stock received minus the discounted strike paid, each weighted by its exercise probability. d1 and d2 are the log-moneyness of the forward, measured in standard deviations of the log price (σ·√T), with a σ²/2 shift between them.

**Why `erfc`.** N(x) = ½·erfc(−x/√2). The forms `1 − N(−x)` and `½(1 + erf(x/√2))` subtract nearly equal numbers in the lower tail, so all relative precision is lost (N(−10) ≈ 7.6e-24 comes out as 0 or noise). `erfc` is evaluated directly in the tail. Deep out-of-the-money prices are made of these tail values, so they stay accurate.

**T → 0.** The option is worth its intrinsic value, max(S − K, 0) or max(K − S, 0). This is handled first because σ·√T is also zero there.

**σ → 0.** The terminal price collapses onto the forward S·e^((r−q)T), so the payoff is deterministic and the value is the discounted forward intrinsic value, max(S·e^(−qT) − K·e^(−rT), 0) for a call. d1 and d2 diverge as σ·√T → 0, so below `MIN_TOTAL_VOL = 1e-12` the limit is returned directly instead of dividing by a near-zero number.

## Design choices

- `BsInputs` has private fields and one validating constructor, so an invalid value cannot exist and `price` can be infallible. It has no `Deserialize`, since that would bypass validation. `with_spot`, `with_volatility` and `with_time_to_expiry` re-validate, for bumps and scenarios later.
- `Terms` (crate-private) computes d1, d2, √T and both discount factors once. Part 3 reuses it for Greeks. `d` is an `Option` so nobody can read d1 and d2 in the degenerate regime.
- The price is clamped at zero. The exact value is never negative, but deep OTM the two terms nearly cancel and rounding can leave about −1e-30.
- `serde` is an optional feature on `OptionType` only, so `pricing` stays dependency-light.

## Interview questions

1. **Why not compute N(x) as 1 − N(−x)?** Cancellation. In the tail the result is tiny and the subtraction discards its leading digits, whereas `erfc` keeps relative precision.
2. **How do you know the implementation is right?** Reference prices from Hull and Haug plus mpmath values at 1e-10, put-call parity and no-arbitrage bounds under `proptest`, monotonicity on a grid, and the T = 0 and σ → 0 limits.
3. **What does N(d2) mean?** The risk-neutral probability of finishing in the money. N(d1) is the probability under the stock-numeraire measure and also the option's delta when q = 0 (Part 3).
