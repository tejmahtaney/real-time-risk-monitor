//! The correlated geometric Brownian motion simulator with mean-reverting log vol.

use chrono::{DateTime, Utc};
use domain::{ClockStep, StepKind, Symbol};
use nalgebra::{DMatrix, DVector};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::StandardNormal;

use crate::feed::{FeedError, MarketTick, PriceFeed, Quote};
use crate::seed::SeedData;
use crate::simulation_config::SimulationConfig;

/// Shrinkage increment: λ goes 0.05, 0.10, … until the Cholesky factor exists.
const SHRINKAGE_STEP: f64 = 0.05;

/// Tolerance for the unit diagonal and the symmetry of the correlation matrix.
const MATRIX_TOLERANCE: f64 = 1e-9;

/// One symbol's simulated state.
#[derive(Debug, Clone)]
struct SymbolState {
    symbol: Symbol,
    spot: f64,
    /// x = ln σ, the current log vol.
    log_vol: f64,
    /// x̄ = ln of the seed vol, the level the log vol reverts to.
    log_vol_mean: f64,
}

/// A deterministic price feed: correlated GBM for spots, an Ornstein-Uhlenbeck process for
/// each symbol's log vol.
///
/// **Prices.** With h = `trading_years` of the step, each spot moves by the exact log step
///
/// Sₜ₊ₕ = Sₜ · exp((μ − ½σ²)·h + σ·√h·Z)
///
/// where σ is the symbol's vol at the start of the step. It is exact because the solution
/// of dS = μS dt + σS dW is log-normal for constant σ: ln S is Brownian motion with drift
/// μ − ½σ², so its increment over h is exactly normal with that mean and variance σ²·h.
/// There is no Euler discretisation error, so the step size does not matter, and S stays
/// positive by construction. Z = L·ε, where L is the Cholesky factor of the calibrated
/// correlation matrix and ε is a vector of independent standard normals, so Z has unit
/// variances and the correlations of the matrix. If the matrix is not positive definite it
/// is shrunk towards the identity, C′ = (1 − λ)·C + λ·I, raising λ by 0.05 until the
/// factorisation succeeds, and a warning gives the final λ.
///
/// **Vol.** x = ln σ reverts to x̄ = ln of the seed vol with speed κ and vol of vol ξ, using the exact
/// Ornstein-Uhlenbeck transition
///
/// xₜ₊ₕ = x̄ + (xₜ − x̄)·e^(−κh) + ξ·√((1 − e^(−2κh)) / (2κ))·W
///
/// with W a standard normal correlated with the same symbol's price shock at ρ
/// (W = ρ·Z + √(1 − ρ²)·η, η independent). Negative ρ gives the leverage effect.
/// The process is on log vol so σ = eˣ is always positive, and shocks are symmetric in
/// logs, so vol is right-skewed as it is in real markets.
///
/// **Overnight.** A step with no trading time (an overnight jump, or a paused clock) leaves
/// spot and vol unchanged and draws no random numbers. Real markets gap overnight on news,
/// so this is a simplification.
///
/// The random stream is `ChaCha8Rng` seeded from the configuration, so the same seed gives
/// the same paths. The exact numbers also depend on the `rand_distr` version, so a
/// dependency upgrade can change them.
#[derive(Debug, Clone)]
pub struct GbmSimulator {
    symbols: Vec<SymbolState>,
    /// Lower Cholesky factor L of the (possibly shrunk) correlation matrix.
    chol: DMatrix<f64>,
    shrinkage: f64,
    rng: ChaCha8Rng,
    drift: f64,
    kappa: f64,
    vol_of_vol: f64,
    spot_vol_correlation: f64,
    // Scratch buffers, so a step allocates nothing after the first.
    eps: DVector<f64>,
    eta: DVector<f64>,
    z: DVector<f64>,
    next: Vec<(f64, f64)>,
}

impl GbmSimulator {
    /// Builds the simulator from the calibrated seed and the configuration. Symbols are in
    /// `seed.symbols` order, which is the order of the correlation matrix.
    ///
    /// # Errors
    ///
    /// [`FeedError::Config`] for an invalid configuration, [`FeedError::EmptySeed`] for no
    /// symbols, [`FeedError::InvalidSeed`] for a missing symbol or a spot or vol that is
    /// not positive and finite, and [`FeedError::InvalidCorrelation`] for a malformed matrix.
    pub fn new(seed: &SeedData, config: &SimulationConfig) -> Result<Self, FeedError> {
        config.validate()?;
        if seed.symbols.is_empty() {
            return Err(FeedError::EmptySeed);
        }
        let mut symbols = Vec::with_capacity(seed.symbols.len());
        for symbol in &seed.symbols {
            let invalid = |message: String| FeedError::InvalidSeed {
                symbol: symbol.clone(),
                message,
            };
            let entry = seed
                .underlyings
                .get(symbol)
                .ok_or_else(|| invalid("no calibration for the symbol".into()))?;
            for (name, value) in [("spot", entry.spot), ("vol", entry.vol)] {
                if !value.is_finite() || value <= 0.0 {
                    return Err(invalid(format!(
                        "{name} must be positive and finite, got {value}"
                    )));
                }
            }
            symbols.push(SymbolState {
                symbol: symbol.clone(),
                spot: entry.spot,
                log_vol: entry.vol.ln(),
                log_vol_mean: entry.vol.ln(),
            });
        }
        let n = symbols.len();
        let correlation = correlation_matrix(&seed.correlation, n)?;
        let (chol, shrinkage) = cholesky_with_shrinkage(&correlation);
        Ok(Self {
            symbols,
            chol,
            shrinkage,
            rng: ChaCha8Rng::seed_from_u64(config.seed),
            drift: config.drift,
            kappa: config.vol_process.mean_reversion,
            vol_of_vol: config.vol_process.vol_of_vol,
            spot_vol_correlation: config.vol_process.spot_vol_correlation,
            eps: DVector::zeros(n),
            eta: DVector::zeros(n),
            z: DVector::zeros(n),
            next: Vec::with_capacity(n),
        })
    }

    /// The weight λ used to shrink the correlation matrix towards the identity. Zero when
    /// the calibrated matrix was positive definite.
    pub fn correlation_shrinkage(&self) -> f64 {
        self.shrinkage
    }

    /// The current prices as a tick stamped `time` and `kind`, without moving anything.
    /// The engine uses it for the state before the first step.
    pub fn current_tick(&self, time: DateTime<Utc>, kind: StepKind) -> MarketTick {
        MarketTick {
            time,
            kind,
            quotes: self
                .symbols
                .iter()
                .map(|s| {
                    (
                        s.symbol.clone(),
                        Quote {
                            spot: s.spot,
                            vol: s.log_vol.exp(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Draws the shocks and moves every symbol by `h` trading years. Nothing changes if any
    /// new value is not positive and finite.
    fn diffuse(&mut self, h: f64) -> Result<(), FeedError> {
        for e in self.eps.iter_mut() {
            *e = self.rng.sample(StandardNormal);
        }
        for e in self.eta.iter_mut() {
            *e = self.rng.sample(StandardNormal);
        }
        // z = L · eps: correlated price shocks with unit variance.
        self.z.gemv(1.0, &self.chol, &self.eps, 0.0);

        let rho = self.spot_vol_correlation;
        let rho_perp = (1.0 - rho * rho).max(0.0).sqrt();
        let sqrt_h = h.sqrt();
        let decay = (-self.kappa * h).exp();
        // 1 − e^(−2κh) through expm1, which stays accurate for small κh.
        let ou_sd =
            self.vol_of_vol * (-(-2.0 * self.kappa * h).exp_m1() / (2.0 * self.kappa)).sqrt();

        self.next.clear();
        for (state, (&z, &eta)) in self.symbols.iter().zip(self.z.iter().zip(self.eta.iter())) {
            let sigma = state.log_vol.exp();
            let w = rho * z + rho_perp * eta;
            let spot =
                state.spot * ((self.drift - 0.5 * sigma * sigma) * h + sigma * sqrt_h * z).exp();
            let log_vol =
                state.log_vol_mean + (state.log_vol - state.log_vol_mean) * decay + ou_sd * w;
            let vol = log_vol.exp();
            for (quantity, value) in [("spot", spot), ("vol", vol)] {
                if !value.is_finite() || value <= 0.0 {
                    return Err(FeedError::NonFinite {
                        symbol: state.symbol.clone(),
                        quantity,
                        value,
                    });
                }
            }
            self.next.push((spot, log_vol));
        }
        for (state, &(spot, log_vol)) in self.symbols.iter_mut().zip(&self.next) {
            state.spot = spot;
            state.log_vol = log_vol;
        }
        Ok(())
    }
}

impl PriceFeed for GbmSimulator {
    fn step(&mut self, step: &ClockStep) -> Result<MarketTick, FeedError> {
        let h = step.trading_years;
        if !h.is_finite() || h < 0.0 {
            return Err(FeedError::InvalidStep(format!(
                "trading_years must be finite and not negative, got {h}"
            )));
        }
        if step.kind == StepKind::Overnight && h > 0.0 {
            return Err(FeedError::InvalidStep(format!(
                "an overnight step has no trading time, got {h}"
            )));
        }
        if h > 0.0 {
            self.diffuse(h)?;
        }
        Ok(self.current_tick(step.to, step.kind))
    }
}

/// Checks the matrix is n × n, finite, symmetric with a unit diagonal and entries in [−1, 1],
/// and returns it as a `DMatrix`.
fn correlation_matrix(rows: &[Vec<f64>], n: usize) -> Result<DMatrix<f64>, FeedError> {
    let bad = |msg: String| Err(FeedError::InvalidCorrelation(msg));
    if rows.len() != n {
        return bad(format!("expected {n} rows, got {}", rows.len()));
    }
    for (i, row) in rows.iter().enumerate() {
        if row.len() != n {
            return bad(format!("row {i} has {} entries, expected {n}", row.len()));
        }
        for (j, &value) in row.iter().enumerate() {
            if !value.is_finite() {
                return bad(format!("entry ({i}, {j}) is {value}"));
            }
            if value.abs() > 1.0 + MATRIX_TOLERANCE {
                return bad(format!("entry ({i}, {j}) is {value}, outside [-1, 1]"));
            }
            if i == j && (value - 1.0).abs() > MATRIX_TOLERANCE {
                return bad(format!("diagonal entry {i} is {value}, expected 1"));
            }
            let mirror = rows
                .get(j)
                .and_then(|r| r.get(i))
                .copied()
                .unwrap_or(f64::NAN);
            if (value - mirror).abs() > MATRIX_TOLERANCE {
                return bad(format!("entries ({i}, {j}) and ({j}, {i}) differ"));
            }
        }
    }
    Ok(DMatrix::from_fn(n, n, |i, j| {
        rows.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0)
    }))
}

/// The lower Cholesky factor of `matrix`, shrinking it towards the identity if it is not
/// positive definite, together with the weight λ used.
///
/// Tries C, then C′ = (1 − λ)·C + λ·I for λ = 0.05, 0.10, … up to 1, where C′ is the
/// identity and always factorises. A warning gives the final λ when it is not zero.
fn cholesky_with_shrinkage(matrix: &DMatrix<f64>) -> (DMatrix<f64>, f64) {
    let n = matrix.nrows();
    let identity = DMatrix::<f64>::identity(n, n);
    for step in 0..=20_u32 {
        let lambda = (f64::from(step) * SHRINKAGE_STEP).min(1.0);
        let shrunk = if step == 0 {
            matrix.clone()
        } else {
            matrix * (1.0 - lambda) + &identity * lambda
        };
        if let Some(factor) = shrunk.cholesky() {
            if step > 0 {
                tracing::warn!(
                    shrinkage = lambda,
                    "correlation matrix is not positive definite, shrunk towards the identity"
                );
            }
            return (factor.l(), lambda);
        }
    }
    (identity, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use approx::{assert_abs_diff_eq, assert_relative_eq};
    use chrono::{NaiveDate, TimeZone};

    use super::*;
    use crate::loader::DataSource;
    use crate::seed::SymbolSeed;
    use crate::simulation_config::VolProcessConfig;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn seed_data(specs: &[(&str, f64, f64)], correlation: Vec<Vec<f64>>) -> SeedData {
        let mut underlyings = BTreeMap::new();
        for &(name, spot, vol) in specs {
            underlyings.insert(
                sym(name),
                SymbolSeed {
                    spot,
                    vol_realised: vol,
                    vol_ewma: vol,
                    vol,
                    dividend_yield: 0.01,
                    source: DataSource::Fallback {
                        file: format!("{name}.csv"),
                    },
                },
            );
        }
        SeedData {
            as_of: NaiveDate::from_ymd_opt(2026, 9, 25).unwrap(),
            rate: 0.04,
            symbols: specs.iter().map(|&(name, _, _)| sym(name)).collect(),
            underlyings,
            return_dates: Vec::new(),
            returns: Vec::new(),
            correlation,
        }
    }

    fn config(seed: u64, drift: f64, kappa: f64, xi: f64, rho: f64) -> SimulationConfig {
        SimulationConfig {
            seed,
            tick_interval_ms: 250,
            speed: 120.0,
            drift,
            start_date: None,
            vol_process: VolProcessConfig {
                mean_reversion: kappa,
                vol_of_vol: xi,
                spot_vol_correlation: rho,
            },
        }
    }

    fn one_symbol(vol: f64, cfg: &SimulationConfig) -> GbmSimulator {
        GbmSimulator::new(&seed_data(&[("SPY", 100.0, vol)], vec![vec![1.0]]), cfg).unwrap()
    }

    fn two_symbols(rho: f64, cfg: &SimulationConfig) -> GbmSimulator {
        let seed = seed_data(
            &[("AAPL", 200.0, 0.3), ("SPY", 500.0, 0.15)],
            vec![vec![1.0, rho], vec![rho, 1.0]],
        );
        GbmSimulator::new(&seed, cfg).unwrap()
    }

    fn five_symbols(cfg: &SimulationConfig) -> GbmSimulator {
        let specs = [
            ("AAPL", 340.0, 0.28),
            ("MSFT", 516.0, 0.38),
            ("NVDA", 225.0, 0.39),
            ("SPY", 771.0, 0.11),
            ("TSLA", 372.0, 0.54),
        ];
        let corr = (0..5)
            .map(|i| {
                (0..5)
                    .map(|j| if i == j { 1.0 } else { 0.3 })
                    .collect::<Vec<f64>>()
            })
            .collect();
        GbmSimulator::new(&seed_data(&specs, corr), cfg).unwrap()
    }

    fn step_of(h: f64) -> ClockStep {
        let from = Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap();
        ClockStep {
            from,
            to: from + chrono::Duration::seconds(30),
            trading_years: h,
            calendar_years: 0.0,
            session_date: NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
            kind: StepKind::Intraday,
        }
    }

    fn overnight() -> ClockStep {
        ClockStep {
            trading_years: 0.0,
            kind: StepKind::Overnight,
            ..step_of(0.0)
        }
    }

    fn quote(tick: &MarketTick, name: &str) -> Quote {
        tick.quotes[&sym(name)]
    }

    fn mean(xs: &[f64]) -> f64 {
        xs.iter().sum::<f64>() / xs.len() as f64
    }

    fn variance(xs: &[f64]) -> f64 {
        let m = mean(xs);
        xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64
    }

    fn correlation(a: &[f64], b: &[f64]) -> f64 {
        let (ma, mb) = (mean(a), mean(b));
        let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let va: f64 = a.iter().map(|x| (x - ma).powi(2)).sum();
        let vb: f64 = b.iter().map(|y| (y - mb).powi(2)).sum();
        cov / (va * vb).sqrt()
    }

    fn path(sim: &mut GbmSimulator, steps: usize, h: f64) -> Vec<f64> {
        let mut out = Vec::new();
        for _ in 0..steps {
            let tick = sim.step(&step_of(h)).unwrap();
            for q in tick.quotes.values() {
                out.push(q.spot);
                out.push(q.vol);
            }
        }
        out
    }

    #[test]
    fn same_seed_same_path_and_different_seed_different_path() {
        let h = 1.0 / (252.0 * 780.0);
        let a = path(
            &mut five_symbols(&config(42, 0.05, 5.0, 0.9, -0.7)),
            3_000,
            h,
        );
        let b = path(
            &mut five_symbols(&config(42, 0.05, 5.0, 0.9, -0.7)),
            3_000,
            h,
        );
        let c = path(
            &mut five_symbols(&config(43, 0.05, 5.0, 0.9, -0.7)),
            3_000,
            h,
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
        // Different seeds diverge from the first step, not only eventually.
        assert_ne!(a[..10], c[..10]);
    }

    #[test]
    fn log_returns_have_the_gbm_mean_and_variance() {
        // With no vol of vol the vol is constant and log returns are exactly normal. The
        // sizes make the −½σ²h correction about 11 standard errors, so dropping it fails.
        let (mu, sigma, h, n) = (0.10, 0.50, 1e-2, 200_000_usize);
        let mut sim = one_symbol(sigma, &config(7, mu, 5.0, 0.0, -0.7));
        let mut prev = quote(
            &sim.current_tick(step_of(0.0).to, StepKind::Intraday),
            "SPY",
        )
        .spot;
        let mut returns = Vec::with_capacity(n);
        for _ in 0..n {
            let spot = quote(&sim.step(&step_of(h)).unwrap(), "SPY").spot;
            returns.push((spot / prev).ln());
            prev = spot;
        }
        let nf = n as f64;
        let expected_mean = (mu - 0.5 * sigma * sigma) * h;
        let expected_var = sigma * sigma * h;
        let se_mean = (expected_var / nf).sqrt();
        let se_var = expected_var * (2.0 / (nf - 1.0)).sqrt();
        assert!(
            (mean(&returns) - expected_mean).abs() < 4.0 * se_mean,
            "mean {} vs {expected_mean}, se {se_mean}",
            mean(&returns)
        );
        assert!(
            (variance(&returns) - expected_var).abs() < 4.0 * se_var,
            "variance {} vs {expected_var}, se {se_var}",
            variance(&returns)
        );
    }

    #[test]
    fn shocks_have_the_target_correlation() {
        let (h, n) = (1e-3, 100_000_usize);
        let mut sim = two_symbols(0.6, &config(11, 0.0, 5.0, 0.0, -0.7));
        let first = sim.current_tick(step_of(0.0).to, StepKind::Intraday);
        let (mut a, mut b) = (quote(&first, "AAPL").spot, quote(&first, "SPY").spot);
        let (mut ra, mut rb) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for _ in 0..n {
            let tick = sim.step(&step_of(h)).unwrap();
            let (na, nb) = (quote(&tick, "AAPL").spot, quote(&tick, "SPY").spot);
            ra.push((na / a).ln());
            rb.push((nb / b).ln());
            (a, b) = (na, nb);
        }
        let rho = correlation(&ra, &rb);
        assert!((rho - 0.6).abs() < 0.02, "sample correlation {rho}");
        assert_abs_diff_eq!(sim.correlation_shrinkage(), 0.0);
    }

    #[test]
    fn log_vol_follows_the_exact_ornstein_uhlenbeck_law() {
        let (sigma0, mu, kappa, xi, rho) = (0.25, 0.0, 5.0, 0.9, -0.7);
        let (h, n) = (0.01, 200_000_usize);
        let mut sim = one_symbol(sigma0, &config(3, mu, kappa, xi, rho));
        let x_bar = f64::ln(sigma0);
        let decay = f64::exp(-kappa * h);
        let expected_var = xi * xi * (1.0 - f64::exp(-2.0 * kappa * h)) / (2.0 * kappa);

        let first = quote(
            &sim.current_tick(step_of(0.0).to, StepKind::Intraday),
            "SPY",
        );
        let (mut spot, mut vol) = (first.spot, first.vol);
        let (mut xs, mut residuals, mut shocks) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..n {
            let next = quote(&sim.step(&step_of(h)).unwrap(), "SPY");
            assert!(next.vol > 0.0 && next.vol.is_finite(), "vol {}", next.vol);
            let (x, x_next) = (vol.ln(), next.vol.ln());
            // Recover the price shock from the return and the start-of-step vol.
            let log_return = (next.spot / spot).ln();
            shocks.push((log_return - (mu - 0.5 * vol * vol) * h) / (vol * h.sqrt()));
            residuals.push(x_next - x_bar - (x - x_bar) * decay);
            xs.push(x_next);
            (spot, vol) = (next.spot, next.vol);
        }
        let nf = n as f64;

        // Long-run level: the stationary sd is ξ/√(2κ) and the autocorrelation time is
        // about 40 steps, so the standard error is near 0.004.
        assert!(
            (mean(&xs) - x_bar).abs() < 0.02,
            "mean ln vol {}",
            mean(&xs)
        );
        assert_relative_eq!(variance(&xs), xi * xi / (2.0 * kappa), max_relative = 0.1);

        // One-step variance of ln σ is the exact OU value.
        let se_mean = (expected_var / nf).sqrt();
        let se_var = expected_var * (2.0 / (nf - 1.0)).sqrt();
        assert!(
            mean(&residuals).abs() < 4.0 * se_mean,
            "mean {}",
            mean(&residuals)
        );
        assert!(
            (variance(&residuals) - expected_var).abs() < 4.0 * se_var,
            "variance {} vs {expected_var}",
            variance(&residuals)
        );

        // Leverage: vol shocks correlate with price shocks at ρ.
        let leverage = correlation(&shocks, &residuals);
        assert!((leverage - rho).abs() < 0.02, "leverage {leverage}");
        // And the price shocks are standard normal.
        assert!(mean(&shocks).abs() < 0.02);
        assert!((variance(&shocks) - 1.0).abs() < 0.02);
    }

    #[test]
    fn zero_trading_time_changes_nothing_and_draws_nothing() {
        let cfg = config(5, 0.03, 5.0, 0.9, -0.7);
        let mut plain = five_symbols(&cfg);
        let mut gappy = five_symbols(&cfg);
        let h = 1e-4;
        let mut plain_ticks = Vec::new();
        for _ in 0..3 {
            plain_ticks.push(plain.step(&step_of(h)).unwrap());
        }
        let mut gappy_ticks = vec![gappy.step(&step_of(h)).unwrap()];
        let night = gappy.step(&overnight()).unwrap();
        assert_eq!(night.kind, StepKind::Overnight);
        assert_eq!(night.quotes, gappy_ticks[0].quotes);
        gappy_ticks.push(gappy.step(&step_of(h)).unwrap());
        let paused = gappy.step(&step_of(0.0)).unwrap();
        assert_eq!(paused.quotes, gappy_ticks[1].quotes);
        gappy_ticks.push(gappy.step(&step_of(h)).unwrap());
        for (p, g) in plain_ticks.iter().zip(&gappy_ticks) {
            assert_eq!(p.quotes, g.quotes);
        }
    }

    #[test]
    fn a_tick_carries_the_step_time_and_kind() {
        let mut sim = one_symbol(0.2, &config(1, 0.0, 5.0, 0.9, -0.7));
        let step = ClockStep {
            kind: StepKind::Close,
            ..step_of(1e-4)
        };
        let tick = sim.step(&step).unwrap();
        assert_eq!(tick.time, step.to);
        assert_eq!(tick.kind, StepKind::Close);
        assert_eq!(tick.quotes.len(), 1);
    }

    #[test]
    fn shrinkage_repairs_a_matrix_that_is_not_positive_definite() {
        // Eigenvalues 2.2 and -0.2. (1 - λ)(-0.2) + λ > 0 needs λ > 1/6, so 0.20 is the
        // first multiple of 0.05 that works.
        let c = DMatrix::from_row_slice(2, 2, &[1.0, 1.2, 1.2, 1.0]);
        let (l, lambda) = cholesky_with_shrinkage(&c);
        assert_relative_eq!(lambda, 0.20, epsilon = 1e-12);
        // L Lᵀ is the shrunk matrix 0.8·C + 0.2·I, whose off-diagonal is 0.96.
        let product = &l * l.transpose();
        assert_relative_eq!(product[(0, 0)], 1.0, epsilon = 1e-12);
        assert_relative_eq!(product[(1, 1)], 1.0, epsilon = 1e-12);
        assert_relative_eq!(product[(0, 1)], 0.96, epsilon = 1e-12);
    }

    #[test]
    fn a_singular_matrix_is_shrunk_once() {
        let c = DMatrix::from_row_slice(2, 2, &[1.0, 1.0, 1.0, 1.0]);
        let (l, lambda) = cholesky_with_shrinkage(&c);
        assert_relative_eq!(lambda, 0.05, epsilon = 1e-12);
        assert_relative_eq!((&l * l.transpose())[(0, 1)], 0.95, epsilon = 1e-12);
    }

    #[test]
    fn a_valid_matrix_is_not_shrunk() {
        let c = DMatrix::from_row_slice(2, 2, &[1.0, 0.5, 0.5, 1.0]);
        let (l, lambda) = cholesky_with_shrinkage(&c);
        assert_abs_diff_eq!(lambda, 0.0);
        let product = &l * l.transpose();
        assert_relative_eq!(product[(0, 1)], 0.5, epsilon = 1e-12);
        assert_relative_eq!(product[(1, 0)], 0.5, epsilon = 1e-12);
    }

    #[test]
    fn cholesky_turns_independent_shocks_into_correlated_ones() {
        // For 2×2 with correlation ρ, L = [[1, 0], [ρ, √(1 − ρ²)]].
        let c = DMatrix::from_row_slice(2, 2, &[1.0, 0.6, 0.6, 1.0]);
        let (l, _) = cholesky_with_shrinkage(&c);
        assert_relative_eq!(l[(0, 0)], 1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(l[(0, 1)], 0.0);
        assert_relative_eq!(l[(1, 0)], 0.6, epsilon = 1e-12);
        assert_relative_eq!(l[(1, 1)], 0.8, epsilon = 1e-12);
    }

    #[test]
    fn the_simulator_shrinks_an_indefinite_calibrated_matrix() {
        // Pairwise correlations that cannot all hold at once.
        let rows = vec![
            vec![1.0, 0.9, -0.9],
            vec![0.9, 1.0, 0.9],
            vec![-0.9, 0.9, 1.0],
        ];
        let seed = seed_data(
            &[
                ("AAPL", 100.0, 0.2),
                ("MSFT", 100.0, 0.2),
                ("SPY", 100.0, 0.2),
            ],
            rows.clone(),
        );
        let sim = GbmSimulator::new(&seed, &config(1, 0.0, 5.0, 0.9, -0.7)).unwrap();
        // (1 - λ)·e_min + λ > 0, where e_min is the smallest eigenvalue.
        let matrix = DMatrix::from_fn(3, 3, |i, j| rows[i][j]);
        let e_min = matrix.symmetric_eigenvalues().min();
        assert!(e_min < 0.0);
        let needed = -e_min / (1.0 - e_min);
        let expected = ((needed / SHRINKAGE_STEP).floor() + 1.0) * SHRINKAGE_STEP;
        assert_relative_eq!(sim.correlation_shrinkage(), expected, epsilon = 1e-9);
        // The shrunk matrix still has a unit diagonal, so shocks keep unit variance.
        let product = &sim.chol * sim.chol.transpose();
        for i in 0..3 {
            assert_relative_eq!(product[(i, i)], 1.0, epsilon = 1e-12);
        }
    }

    #[test]
    fn bad_inputs_are_rejected_when_building() {
        let ok_cfg = config(1, 0.0, 5.0, 0.9, -0.7);
        let two = |rows: Vec<Vec<f64>>| {
            GbmSimulator::new(
                &seed_data(&[("AAPL", 100.0, 0.2), ("SPY", 100.0, 0.2)], rows),
                &ok_cfg,
            )
        };
        let nan = f64::NAN;
        let bad_matrices = [
            vec![vec![1.0, 0.5], vec![0.4, 1.0]],
            vec![vec![1.0, 0.5]],
            vec![vec![1.0, 0.5, 0.0], vec![0.5, 1.0]],
            vec![vec![1.0, nan], vec![nan, 1.0]],
            vec![vec![0.9, 0.5], vec![0.5, 1.0]],
            vec![vec![1.0, 1.5], vec![1.5, 1.0]],
        ];
        for (i, rows) in bad_matrices.into_iter().enumerate() {
            assert!(
                matches!(two(rows), Err(FeedError::InvalidCorrelation(_))),
                "matrix {i}"
            );
        }

        let empty = seed_data(&[], Vec::new());
        assert!(matches!(
            GbmSimulator::new(&empty, &ok_cfg),
            Err(FeedError::EmptySeed)
        ));
        for (spot, vol) in [
            (0.0, 0.2),
            (-5.0, 0.2),
            (100.0, 0.0),
            (nan, 0.2),
            (100.0, f64::INFINITY),
        ] {
            let seed = seed_data(&[("SPY", spot, vol)], vec![vec![1.0]]);
            assert!(
                matches!(
                    GbmSimulator::new(&seed, &ok_cfg),
                    Err(FeedError::InvalidSeed { .. })
                ),
                "spot {spot} vol {vol}"
            );
        }
        let mut missing = seed_data(&[("SPY", 100.0, 0.2)], vec![vec![1.0]]);
        missing.underlyings.clear();
        assert!(matches!(
            GbmSimulator::new(&missing, &ok_cfg),
            Err(FeedError::InvalidSeed { .. })
        ));
        let seed = seed_data(&[("SPY", 100.0, 0.2)], vec![vec![1.0]]);
        let bad_cfg = config(1, 0.0, 5.0, 0.9, 2.0);
        assert!(matches!(
            GbmSimulator::new(&seed, &bad_cfg),
            Err(FeedError::Config(_))
        ));
    }

    #[test]
    fn invalid_steps_are_rejected_and_change_nothing() {
        let mut sim = five_symbols(&config(9, 0.0, 5.0, 0.9, -0.7));
        sim.step(&step_of(1e-4)).unwrap();
        let before = sim.current_tick(step_of(0.0).to, StepKind::Intraday);
        let bad_overnight = ClockStep {
            trading_years: 1e-4,
            kind: StepKind::Overnight,
            ..step_of(0.0)
        };
        for step in [
            step_of(-1e-4),
            step_of(f64::NAN),
            step_of(f64::INFINITY),
            bad_overnight,
        ] {
            assert!(matches!(sim.step(&step), Err(FeedError::InvalidStep(_))));
        }
        // A step so long that exp overflows or underflows is caught, not turned into NaN.
        assert!(matches!(
            sim.step(&step_of(1e9)),
            Err(FeedError::NonFinite { .. })
        ));
        assert_eq!(
            sim.current_tick(step_of(0.0).to, StepKind::Intraday),
            before
        );
    }

    #[test]
    fn a_tick_updates_a_market_state() {
        let seed = seed_data(
            &[("AAPL", 200.0, 0.3), ("SPY", 500.0, 0.15)],
            vec![vec![1.0, 0.6], vec![0.6, 1.0]],
        );
        let start = Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap();
        let mut market = seed.to_market_state(start).unwrap();
        let mut sim = GbmSimulator::new(&seed, &config(2, 0.0, 5.0, 0.9, -0.7)).unwrap();
        let step = step_of(1e-3);
        let tick = sim.step(&step).unwrap();
        tick.apply_to(&mut market).unwrap();
        assert_eq!(market.as_of(), step.to);
        for (symbol, q) in &tick.quotes {
            let state = market.underlying(symbol).unwrap();
            assert_relative_eq!(state.spot(), q.spot);
            assert_relative_eq!(state.vol(), q.vol);
            assert_relative_eq!(state.dividend_yield(), 0.01);
        }
    }

    #[test]
    fn the_simulator_works_as_a_dyn_price_feed() {
        let mut sim = one_symbol(0.2, &config(1, 0.0, 5.0, 0.9, -0.7));
        let feed: &mut dyn PriceFeed = &mut sim;
        assert!(feed.step(&step_of(1e-4)).is_ok());
    }
}
