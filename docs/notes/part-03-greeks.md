# Part 03 - Greeks

## Key maths

With φ the standard normal pdf, N the cdf, d1 and d2 as in Part 2, and dq = e^(−qT), dr = e^(−rT):

- Delta: call dq·N(d1), put −dq·N(−d1). Gamma: dq·φ(d1) / (S·σ·√T). Vega: S·dq·φ(d1)·√T.
- Theta (per year, = −∂V/∂T): −S·dq·φ(d1)·σ/(2√T) ∓ r·K·dr·N(±d2) ± q·S·dq·N(±d1), upper sign for the call.
- Rho: call K·T·dr·N(d2), put −K·T·dr·N(−d2). Vanna: −dq·φ(d1)·d2/σ. Volga: vega·d1·d2/σ.
- Only the sign ω = ±1 differs between call and put, so one code path serves both. The signed CDFs N(ω·d1) and N(ω·d2) are computed once and shared by price, delta, theta and rho.

**What each means for a trader.**

- Delta is the share-equivalent exposure. Gamma is how fast delta changes, so it is the cost of staying hedged: long gamma gains on big moves.
- Vega is the P&L for a one-unit rise in implied vol. Theta is what the position earns or bleeds as a day passes, and long gamma pays for it.
- Rho is the rate sensitivity and is small for short-dated equity options.
- Vanna is how delta shifts when vol moves, which matters for skew hedging. Volga is the convexity of vega, so it makes a long-vol book gain more than linearly on a vol spike.

**Why gamma peaks at the money near expiry.** Gamma ∝ φ(d1) / (S·σ·√T). At the money d1 ≈ 0, so φ is at its maximum of about 0.399. The 1/√T factor grows as T → 0, so the peak gets taller and narrower: delta swings from 0 to 1 across an ever thinner band of spot, which is the kink in the payoff. Away from the money d1 grows like |ln(S/K)|/(σ√T), so φ(d1) collapses faster than 1/√T rises and gamma goes to zero instead.

## Design choices

- One `Terms` helper holds d1, d2, √T and the discount factors. `Terms::value` builds the price, so `price()` and `greeks().price` are the same expression and cannot drift apart.
- Put delta is −dq·N(−d1). The form dq·(N(d1) − 1) subtracts two numbers within rounding of each other for a deep out-of-the-money put and loses every digit.
- Degenerate cases follow the price. At T = 0 only price and delta are non-zero. Below `MIN_TOTAL_VOL` the Greeks are the derivatives of the discounted forward intrinsic value when strictly in the money and zero otherwise.

## Finite-difference bump sizes

A central difference has truncation error ∝ h², falling with h, and round-off ∝ ε·|V|/hⁿ for an n-th derivative, rising as h falls. Sizes sit near the balance.

- Spot bumps scale with S·σ·√T, the standard deviation of the spot move over the option's life, so a short-dated low-vol option is not stepped across its kink by a fixed 0.1 % bump. Gamma divides by h², so it gets 1e-3 where delta gets 1e-4.
- Vega 1e-5, rho 1e-6, theta 1e-6 years, volga and vanna 1e-4 in σ.
- The tolerance is 1e-5 relative, floor 1e-8, plus the unavoidable round-off 8·ε·(S + K)/hⁿ. Volga also takes one Richardson step, because at low vol and six or more deviations out of the money the plain h² truncation exceeds 1e-5. A 1e-5 relative error planted in volga is caught, so the check is not vacuous.

## Benchmark

Release build, `cargo bench -p pricing`: **40.8 ns** for one `greeks()` call and **44.1 ns per option** over 10,000 varied options (441 µs for the batch, 22.7 M options a second). The README will quote this.

## Interview questions

1. **Why is the put delta not written as N(d1) − 1?** Cancellation. For a deep out-of-the-money put N(d1) is within rounding of 1, so the difference keeps no significant digits, whereas N(−d1) comes straight from `erfc`.
2. **How do you know gamma is right?** Three independent checks: mpmath references to 1e-10 for three parameter sets, a central second difference on the price with a spot bump scaled to S·σ·√T, and the call-put identity that gamma is the same for both.
3. **Why does gamma blow up near expiry at the money but not elsewhere?** It is φ(d1)/(S·σ·√T). At the money φ is maximal and 1/√T diverges. Away from it φ(d1) decays exponentially in 1/T and wins.
