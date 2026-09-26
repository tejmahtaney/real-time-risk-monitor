# Part 08 - Price simulator and market clock

## Key maths

- Exact log step, h = trading years: Sₜ₊ₕ = Sₜ·exp((μ − ½σ²)h + σ√h·Z). Z = L·ε, with L the Cholesky factor of the correlation matrix C (L Lᵀ = C) and ε independent standard normals.
- Log vol, x = ln σ, is an Ornstein-Uhlenbeck process: xₜ₊ₕ = x̄ + (xₜ − x̄)e^(−κh) + ξ√((1 − e^(−2κh))/(2κ))·W. The one-step variance is ξ²(1 − e^(−2κh))/(2κ) and the stationary variance is ξ²/(2κ). W = ρ·Z + √(1 − ρ²)·η, so corr(W, Z) = ρ.
- Prices use the vol at the start of the step. If C is not positive definite, C′ = (1 − λ)C + λI with λ = 0.05, 0.10, … until Cholesky works. The diagonal stays 1, so shocks keep unit variance.

## Why the log step is exact

For constant σ, S is log-normal: ln S is Brownian motion with drift μ − ½σ², so the increment over any h is exactly normal with mean (μ − ½σ²)h and variance σ²h. There is no Euler error, so a 30-second step and a one-day step give the same distribution, and S can never go negative. The −½σ² term is Itô's correction, the reason the median return is below the mean.

## What Cholesky does to independent shocks

Independent ε has covariance I. L·ε has covariance L I Lᵀ = L Lᵀ = C, so it has exactly the target correlations, and each entry still has variance 1 because C has a unit diagonal. The first asset gets ε₁ unchanged, the second is ρε₁ + √(1 − ρ²)ε₂, and so on down the triangle.

## Why vol mean-reverts in logs

σ = eˣ is positive for any x, so vol cannot go negative (an OU process on σ itself can). Shocks are symmetric in x, so σ is right-skewed: it spikes up more than it falls, as real vol does. The exact OU transition holds for any step size. Reverting to the calibrated vol keeps a long run from wandering off.

## Trading time versus calendar time

Vols are annualised over 252 trading days of 6.5 hours, so diffusion uses `trading_years` = trading seconds / (252 × 6.5 × 3,600). Option time to expiry and theta use ACT/365 `calendar_years`. The overnight or weekend step has no trading time, so nothing diffuses, but its calendar time is large (65.5 h from Friday to Monday), so Monday's first tick carries the weekend's theta. The clock works in New York local time, so daylight saving changes the gap by an hour and never the 6.5-hour session.

## Design choices

- Synchronous and step-based: the engine (Part 11) and backtester (Part 20) drive the same code.
- A step with no trading time draws no random numbers, so pauses and overnights do not change the path.
- Simulated time is rounded to milliseconds, so the step's end time and both year fractions agree exactly.
- `ChaCha8Rng` is portable and fast, and a config seed reproduces a run exactly. The stream also depends on the `rand_distr` version.

## Interview questions

1. **Why not Euler for the price?** It has discretisation error and can go negative. The exact log step is as cheap and has neither problem.
2. **Why is the correlation matrix sometimes not positive definite, and what do you do?** Pairwise estimates from a short window need not fit one joint distribution. Shrinking towards the identity is a convex combination with a valid matrix, so it is positive definite from some λ on, and it keeps unit variances.
3. **Why does a negative price-vol correlation matter?** It gives the leverage effect: falling prices come with rising vol, so the return distribution is skewed and out-of-the-money puts are worth more than calls.
