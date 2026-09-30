//! Value at Risk and Expected Shortfall over a one-day horizon (Part 18).
//!
//! All three methods apply an **instant shock with no time decay**: they answer
//! "what if this happened right now", not "what if a day passes and then this
//! happens". Daily vol is always the current simulated vol ÷ √252. Correlation
//! is Part 7's calibrated matrix, from the same [`marketdata::SeedData`] the
//! simulator was seeded from (passed in as plain data at construction, since
//! `risk` cannot depend on `marketdata` — see [`cholesky`]'s doc comment for
//! why the same is true of the Cholesky factor). **`VaR` and ES are reported as
//! positive numbers for losses**: a `var_99` of 10,000 means "a 1-in-100 day
//! loses at least $10,000", not a $10,000 gain.

mod cholesky;

use std::collections::BTreeMap;
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{MarketState, Symbol};
use portfolio::{PositionBook, PositionStatus};
use pricing::normal;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::StandardNormal;
use serde::{Deserialize, Serialize};

use crate::summary::RiskSummary;

/// Trading days per year, for `daily_vol = vol / sqrt(TRADING_DAYS_PER_YEAR)`. Duplicated
/// from `marketdata::calibration`'s constant of the same value, for the same
/// dependency-direction reason [`cholesky`] duplicates its Cholesky decomposition.
const TRADING_DAYS_PER_YEAR: f64 = 252.0;

/// Number of delta-gamma Monte Carlo scenarios.
const MC_SCENARIOS: usize = 10_000;

/// A fixed seed for the delta-gamma Monte Carlo's common random numbers. Arbitrary, but
/// fixed and independent of `config/simulation.toml`'s own RNG seed, so the scenario set
/// never changes even if that seed does.
const MC_SEED: u64 = 373_857_339_960;

/// At most 99 historical scenarios (100 closes → 99 returns), per Part 7.
const MAX_HISTORICAL_SCENARIOS: usize = 99;

/// If the median (cache-rebuild-excluded) total compute time over the first
/// [`WARM_UP_TICKS`] computed ticks is over this many microseconds, `VaR` is throttled to
/// every `N`th tick for the rest of the session. See [`decide_recompute_every_ticks`].
const BUDGET_MICROS: u64 = 1_000;

/// How many computed ticks' timings to collect before deciding whether (and how much) to
/// throttle. See [`decide_recompute_every_ticks`].
const WARM_UP_TICKS: usize = 20;

/// Which of the three `VaR` methods a [`VarReport`]'s headline figure comes from, and what
/// `config/risk.toml`'s `headline_var_method` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum VarMethod {
    /// Closed-form: `sd = sqrt(deltaᵀ · Σ · delta)`, `VaR_α = z_α · sd`.
    DeltaNormal,
    /// 10,000-scenario delta-gamma Monte Carlo with common random numbers.
    #[default]
    DeltaGammaMc,
    /// Full BSM revaluation under each historical return in Part 7's window.
    Historical,
}

/// One method's `VaR95`, `VaR99` and ES97.5, plus its own scenario count, historical window
/// and compute time.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct VarMethodReport {
    /// 95 % one-day `VaR`: a positive number for a loss.
    pub var_95: f64,
    /// 99 % one-day `VaR`: a positive number for a loss.
    pub var_99: f64,
    /// 97.5 % one-day Expected Shortfall (the Basel FRTB level): a positive number for a
    /// loss, always `>= var_99`.
    pub es_97_5: f64,
    /// Scenarios behind this figure: `0` for the closed-form delta-normal method,
    /// `10,000` for delta-gamma Monte Carlo, and the number of historical return dates
    /// used (at most 99) for the historical method.
    pub scenario_count: usize,
    /// The oldest historical return date used, or `None` for the two non-historical
    /// methods.
    pub window_start: Option<NaiveDate>,
    /// The newest historical return date used, or `None` for the two non-historical
    /// methods.
    pub window_end: Option<NaiveDate>,
    /// Microseconds spent computing this one method (including a Monte Carlo cache
    /// rebuild on the tick that triggers one, for [`VarMethod::DeltaGammaMc`]).
    pub compute_micros: u64,
}

/// All three `VaR` methods' results for one tick.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct VarReport {
    /// Which method [`Self::headline`] reads from.
    pub headline_method: VarMethod,
    /// Delta-normal (closed-form).
    pub delta_normal: VarMethodReport,
    /// Delta-gamma Monte Carlo (10,000 scenarios, common random numbers).
    pub delta_gamma_mc: VarMethodReport,
    /// Historical simulation, full revaluation.
    pub historical: VarMethodReport,
    /// Total microseconds spent computing all three methods this tick, including any
    /// one-off Monte Carlo cache rebuild. When `freshly_computed` is `false`, this is the
    /// figure from the tick that actually produced these numbers, not this tick's own
    /// (near-zero) cost.
    pub total_compute_micros: u64,
    /// `false` when this tick's numbers were reused from an earlier tick because `VaR`
    /// compute is throttled to every `recompute_every_ticks`-th tick.
    pub freshly_computed: bool,
    /// `1` while every tick recomputes `VaR` (the default, and the state for the whole
    /// session unless the 1 ms budget is exceeded). Once the warm-up decision (see
    /// [`decide_recompute_every_ticks`]) sets this above `1`, it is fixed for the rest of
    /// the session.
    pub recompute_every_ticks: u32,
}

impl VarReport {
    /// The method named by `headline_method`'s own report — what the dashboard shows as
    /// its headline figure and what `LimitKind::PortfolioVar99` is evaluated against.
    pub fn headline(&self) -> VarMethodReport {
        match self.headline_method {
            VarMethod::DeltaNormal => self.delta_normal,
            VarMethod::DeltaGammaMc => self.delta_gamma_mc,
            VarMethod::Historical => self.historical,
        }
    }
}

/// The median of `samples`, or `0` for an empty slice.
fn median_micros(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        u64::midpoint(sorted[n / 2 - 1], sorted[n / 2])
    }
}

/// Decides, from the (cache-rebuild-excluded) total compute time of [`WARM_UP_TICKS`]
/// computed ticks, how many ticks to skip between recomputations for the rest of the
/// session. `1` means "recompute every tick" (never throttled).
///
/// Called exactly once, when the warm-up sample count first reaches [`WARM_UP_TICKS`],
/// from the *median* of those samples rather than any single one. The very first
/// computed tick always rebuilds the Monte Carlo cache (10,000 × n draws), so timing
/// tick 0 alone — even with rebuild time excluded from each sample — would still risk
/// tripping a sticky throttle on some other one-off warm-up cost (a first allocation, a
/// cold branch predictor) rather than steady-state cost; a median over 20 ticks is not
/// swayed by one spike.
///
/// If the median is over [`BUDGET_MICROS`], `N = ceil(median / BUDGET_MICROS)`, clamped
/// to `2..=10`.
fn decide_recompute_every_ticks(warm_up_samples: &[u64]) -> u32 {
    let median = median_micros(warm_up_samples);
    if median > BUDGET_MICROS {
        // The ratio is always > 1.0 here (median > BUDGET_MICROS > 0), so `ceil()` is
        // always positive and the truncating cast to u32 never loses a sign.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let ticks = (median as f64 / BUDGET_MICROS as f64).ceil() as u32;
        ticks.clamp(2, 10)
    } else {
        1
    }
}

/// Linear interpolation between order statistics: index `i = p · (n − 1)` (fractional,
/// zero-based), interpolating between `sorted_ascending[floor(i)]` and
/// `sorted_ascending[ceil(i)]` — the same definition `NumPy`'s default
/// (`interpolation="linear"`) and Excel's `PERCENTILE.INC` use.
///
/// `sorted_ascending` must be sorted ascending *at every index this call actually reads*
/// — `floor(i)` and `ceil(i)` for the given `p`. The delta-gamma Monte Carlo path relies
/// on this: it only ever calls this with `p >= 0.95`, so it only fully sorts the tail of
/// its scenario array from that point on (see `quantiles_and_es_partial_sort`) and passes
/// the whole (partially sorted) array — the unsorted head is never read because its
/// indices always fall below `floor(0.95 · (n - 1))`.
///
/// Returns `0.0` for an empty slice.
fn interpolated_quantile(sorted_ascending: &[f64], p: f64) -> f64 {
    let n = sorted_ascending.len();
    if n == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let position = p * (n as f64 - 1.0);
    let lower = position.floor();
    let upper = position.ceil();
    let frac = position - lower;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (lower, upper) = (lower as usize, upper as usize);
    if lower == upper {
        sorted_ascending[lower]
    } else {
        sorted_ascending[lower] * (1.0 - frac) + sorted_ascending[upper] * frac
    }
}

/// Mean of every value at or beyond `threshold` (inclusive) — "the average loss at or
/// beyond the `VaR` level". Order does not matter: this filters and averages, it never
/// indexes. Falls back to `threshold` itself if nothing reaches it (possible since
/// `threshold` is interpolated and need not equal any member of the slice) or the slice
/// is empty.
fn tail_mean_at_or_beyond(values: &[f64], threshold: f64) -> f64 {
    let mut sum = 0.0;
    let mut count = 0_u32;
    for &value in values {
        if value >= threshold {
            sum += value;
            count += 1;
        }
    }
    if count == 0 {
        threshold
    } else {
        sum / f64::from(count)
    }
}

/// Partitions `losses` so that indices `floor(0.95·(n-1))..n` are in true ascending
/// order (via `select_nth_unstable_by` then sorting only that tail), then reads `VaR95`,
/// `VaR99` and ES97.5 off it. Every quantile/ES threshold used here is at or above the 95th
/// percentile, so every index [`interpolated_quantile`] and [`tail_mean_at_or_beyond`]
/// ever touch falls inside the now-sorted tail — the unsorted head is never read. This
/// avoids a full `sort_by` of 10,000 elements on every tick.
fn quantiles_and_es_partial_sort(losses: &mut [f64]) -> (f64, f64, f64) {
    let n = losses.len();
    if n == 0 {
        return (0.0, 0.0, 0.0);
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let pivot_index = ((0.95 * (n as f64 - 1.0)).floor() as usize).min(n - 1);
    if pivot_index < n - 1 {
        losses.select_nth_unstable_by(pivot_index, f64::total_cmp);
    }
    losses[pivot_index..].sort_by(f64::total_cmp);

    let var_95 = interpolated_quantile(losses, 0.95);
    let var_99 = interpolated_quantile(losses, 0.99);
    let threshold_97_5 = interpolated_quantile(losses, 0.975);
    let es_97_5 = tail_mean_at_or_beyond(losses, threshold_97_5);
    (var_95, var_99, es_97_5)
}

/// `current simulated vol / sqrt(252)`, or `0.0` if the market has no data for `symbol`
/// (defensive; should not happen for a symbol priced this tick).
fn daily_vol(symbol: &Symbol, market: &MarketState) -> f64 {
    market
        .underlying(symbol)
        .map_or(0.0, |u| u.vol() / TRADING_DAYS_PER_YEAR.sqrt())
}

/// Common random numbers and the Cholesky factor for the currently-active symbol set,
/// regenerated only when that set changes.
struct McCache {
    symbols: Vec<Symbol>,
    /// Lower Cholesky factor of the (shrunk if needed) correlation submatrix for
    /// `symbols`, in `symbols` order.
    chol: Vec<Vec<f64>>,
    /// `MC_SCENARIOS` rows of `symbols.len()` iid standard normal draws.
    epsilon: Vec<Vec<f64>>,
}

/// Stateful Value-at-Risk computation, owned by `Engine` for one session.
///
/// Holds the session's calibrated symbol order, correlation matrix and historical return
/// window (cloned once from `marketdata::SeedData` at construction — calibration happens
/// once at startup, so these never change during a session), the delta-gamma Monte
/// Carlo's cached common random numbers, and the adaptive compute-budget state.
pub struct VarCalculator {
    headline_method: VarMethod,
    index: BTreeMap<Symbol, usize>,
    correlation: Vec<Vec<f64>>,
    returns: Vec<Vec<f64>>,
    return_dates: Vec<NaiveDate>,
    mc_cache: Option<McCache>,
    ticks_seen: u64,
    warm_up_samples: Vec<u64>,
    recompute_every_ticks: u32,
    decided: bool,
    last_report: Option<VarReport>,
}

impl VarCalculator {
    /// Builds a calculator from a session's calibrated seed data.
    ///
    /// Infallible: a `correlation` matrix whose shape does not match `symbols` (or that
    /// is missing a symbol entirely) is treated defensively as "no correlation
    /// information for that pair" (falls back to independence) rather than an error —
    /// `SeedData` of this shape would already have failed `GbmSimulator::new`'s own
    /// validation earlier in the startup pipeline, so this is a defensive fallback, not
    /// an expected path.
    pub fn new(
        symbols: Vec<Symbol>,
        correlation: Vec<Vec<f64>>,
        returns: Vec<Vec<f64>>,
        return_dates: Vec<NaiveDate>,
        headline_method: VarMethod,
    ) -> Self {
        let index = symbols
            .into_iter()
            .enumerate()
            .map(|(i, s)| (s, i))
            .collect();
        Self {
            headline_method,
            index,
            correlation,
            returns,
            return_dates,
            mc_cache: None,
            ticks_seen: 0,
            warm_up_samples: Vec::with_capacity(WARM_UP_TICKS),
            recompute_every_ticks: 1,
            decided: false,
            last_report: None,
        }
    }

    /// Computes all three `VaR` methods for this tick, subject to the adaptive compute
    /// budget.
    ///
    /// Always computes on the very first call, and on every call until
    /// [`WARM_UP_TICKS`] computed ticks have been timed and
    /// [`decide_recompute_every_ticks`] has made its one, permanent decision for the
    /// session. After that, computes only when `tick % recompute_every_ticks == 0`;
    /// every other tick clones the last computed [`VarReport`] with `freshly_computed`
    /// set to `false`.
    pub fn compute(
        &mut self,
        summary: &RiskSummary,
        market: &MarketState,
        book: &PositionBook,
        valuation_time: DateTime<Utc>,
    ) -> VarReport {
        let tick_index = self.ticks_seen;
        self.ticks_seen += 1;

        let due = !self.decided || tick_index % u64::from(self.recompute_every_ticks) == 0;
        if !due {
            let mut report = self.last_report.unwrap_or_default();
            report.freshly_computed = false;
            report.recompute_every_ticks = self.recompute_every_ticks;
            return report;
        }

        let started = Instant::now();
        let delta_normal = self.compute_delta_normal(summary, market);
        let (delta_gamma_mc, rebuild_micros) = self.compute_delta_gamma_mc(summary, market);
        let historical = self.compute_historical(summary, market, book, valuation_time);
        let total_elapsed = micros_since(started);

        if !self.decided {
            let budget_sample = total_elapsed.saturating_sub(rebuild_micros);
            self.warm_up_samples.push(budget_sample);
            if self.warm_up_samples.len() >= WARM_UP_TICKS {
                self.recompute_every_ticks = decide_recompute_every_ticks(&self.warm_up_samples);
                self.decided = true;
            }
        }

        let report = VarReport {
            headline_method: self.headline_method,
            delta_normal,
            delta_gamma_mc,
            historical,
            total_compute_micros: total_elapsed,
            freshly_computed: true,
            recompute_every_ticks: self.recompute_every_ticks,
        };
        self.last_report = Some(report);
        report
    }

    /// The correlation between `a` and `b`: `1.0` when they are the same symbol
    /// (regardless of whether either is calibrated), the calibrated value when both are
    /// known, and `0.0` (independence) for any other pair — including one this
    /// calculator has no calibration for at all, per [`Self::new`]'s doc comment.
    fn rho(&self, a: &Symbol, b: &Symbol) -> f64 {
        if a == b {
            return 1.0;
        }
        match (self.index.get(a), self.index.get(b)) {
            (Some(&i), Some(&j)) => self
                .correlation
                .get(i)
                .and_then(|row| row.get(j))
                .copied()
                .unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// Method 1: delta-normal. `sd = sqrt(deltaᵀ · Σ · delta)` with `Σᵢⱼ = ρᵢⱼ·σᵢ·σⱼ`,
    /// computed as a plain double loop over `summary.by_underlying` (a handful of
    /// underlyings, so no linear-algebra crate is warranted). `VaR_α = Φ⁻¹(α) · sd`.
    /// `ES_97.5 = sd · φ(Φ⁻¹(0.975)) / 0.025` — the normal expected-shortfall formula
    /// for the Basel FRTB level.
    fn compute_delta_normal(&self, summary: &RiskSummary, market: &MarketState) -> VarMethodReport {
        let started = Instant::now();
        let symbols: Vec<&Symbol> = summary.by_underlying.keys().collect();

        let mut variance = 0.0;
        for &a in &symbols {
            let delta_a = summary.by_underlying[a].dollar_delta;
            let vol_a = daily_vol(a, market);
            for &b in &symbols {
                let delta_b = summary.by_underlying[b].dollar_delta;
                let vol_b = daily_vol(b, market);
                variance += delta_a * delta_b * self.rho(a, b) * vol_a * vol_b;
            }
        }
        // Round-off can push a variance that is mathematically exactly zero (e.g. an
        // empty book) very slightly negative.
        let sd = variance.max(0.0).sqrt();

        let var_95 = normal::inv_cdf(0.95) * sd;
        let var_99 = normal::inv_cdf(0.99) * sd;
        let es_97_5 = sd * normal::pdf(normal::inv_cdf(0.975)) / 0.025;

        VarMethodReport {
            var_95,
            var_99,
            es_97_5,
            scenario_count: 0,
            window_start: None,
            window_end: None,
            compute_micros: micros_since(started),
        }
    }

    /// Rebuilds the correlation submatrix, Cholesky factor and common random numbers for
    /// `symbols`, in `symbols` order.
    fn build_mc_cache(&self, symbols: &[Symbol]) -> McCache {
        let n = symbols.len();
        let sub_correlation: Vec<Vec<f64>> = symbols
            .iter()
            .map(|a| symbols.iter().map(|b| self.rho(a, b)).collect())
            .collect();
        let (chol, shrinkage) = cholesky::cholesky_with_shrinkage(&sub_correlation);
        if shrinkage > 0.0 {
            tracing::warn!(
                shrinkage,
                symbols = ?symbols,
                "delta-gamma Monte Carlo correlation submatrix was not positive definite, shrunk towards the identity"
            );
        }

        let mut rng = ChaCha8Rng::seed_from_u64(MC_SEED);
        let epsilon: Vec<Vec<f64>> = (0..MC_SCENARIOS)
            .map(|_| (0..n).map(|_| rng.sample(StandardNormal)).collect())
            .collect();

        McCache {
            symbols: symbols.to_vec(),
            chol,
            epsilon,
        }
    }

    /// Method 2: delta-gamma Monte Carlo. Draws correlated daily returns `r = L · ε`
    /// (`L` the Cholesky factor of the correlation submatrix, `ε` the cached common
    /// random numbers), scaled by daily vol, and revalues each scenario with the
    /// second-order P&L approximation documented on
    /// [`quantiles_and_es_partial_sort`]'s caller. Returns the method's own report and
    /// separately the microseconds spent rebuilding the Monte Carlo cache this call (`0`
    /// if the symbol set was unchanged and no rebuild happened) — the caller excludes
    /// the rebuild figure from the session's compute-budget decision, since it is a
    /// one-off cost tied to a symbol-set change, not the steady-state cost the budget
    /// decision is about.
    fn compute_delta_gamma_mc(
        &mut self,
        summary: &RiskSummary,
        market: &MarketState,
    ) -> (VarMethodReport, u64) {
        let started = Instant::now();
        let symbols: Vec<Symbol> = summary.by_underlying.keys().cloned().collect();

        let needs_rebuild = self.mc_cache.as_ref().is_none_or(|c| c.symbols != symbols);
        let mut rebuild_micros = 0_u64;
        if needs_rebuild {
            let rebuild_started = Instant::now();
            self.mc_cache = Some(self.build_mc_cache(&symbols));
            rebuild_micros = micros_since(rebuild_started);
        }
        // `needs_rebuild` guarantees `mc_cache` is `Some` from here on, either freshly
        // built just above or already present and unchanged — but CLAUDE.md forbids
        // `expect`/`unwrap` outside tests, so this reads as a defensive early return
        // (an empty report) rather than a panic on a path that cannot actually happen.
        let Some(cache) = self.mc_cache.as_ref() else {
            return (VarMethodReport::default(), rebuild_micros);
        };
        let n = symbols.len();

        let daily_vols: Vec<f64> = symbols.iter().map(|s| daily_vol(s, market)).collect();
        let dollar_deltas: Vec<f64> = symbols
            .iter()
            .map(|s| summary.by_underlying[s].dollar_delta)
            .collect();
        let dollar_gammas: Vec<f64> = symbols
            .iter()
            .map(|s| summary.by_underlying[s].dollar_gamma_1pct)
            .collect();

        let mut losses = vec![0.0_f64; MC_SCENARIOS];
        for (scenario, loss) in losses.iter_mut().enumerate() {
            let eps = &cache.epsilon[scenario];
            let mut pnl = 0.0;
            for i in 0..n {
                // z_i = (L * eps)_i; L is lower-triangular, so only k <= i contributes.
                let mut z_i = 0.0;
                for (k, &eps_k) in eps.iter().enumerate().take(i + 1) {
                    z_i += cache.chol[i][k] * eps_k;
                }
                let r_i = daily_vols[i] * z_i;
                // dollar_gamma_1pct := ½·γ·(0.01·S)²·qty·mult is the $ P&L of exactly a
                // 1 % move. For an arbitrary return r, the same second-order term is
                // ½·γ·S²·r²·qty·mult, which equals dollar_gamma_1pct · (r / 0.01)²
                // exactly: dividing by (0.01·S)² and multiplying by (r·S)² cancels the
                // S² and leaves r² / 0.01², i.e. (r / 0.01)² times the 1 %-move figure.
                pnl += dollar_deltas[i] * r_i + dollar_gammas[i] * (r_i / 0.01).powi(2);
            }
            *loss = -pnl;
        }

        let (var_95, var_99, es_97_5) = quantiles_and_es_partial_sort(&mut losses);

        (
            VarMethodReport {
                var_95,
                var_99,
                es_97_5,
                scenario_count: MC_SCENARIOS,
                window_start: None,
                window_end: None,
                compute_micros: micros_since(started),
            },
            rebuild_micros,
        )
    }

    /// The shocked `(symbol, spot, vol)` update for `symbol` on historical return row
    /// `date_index`, or `None` if this calculator has no return data for `symbol`, no
    /// row `date_index`, or the market has no data for `symbol` — any of which leaves
    /// that symbol unshocked in that scenario (see [`Self::compute_historical`]).
    fn shocked_update(
        &self,
        market: &MarketState,
        symbol: &Symbol,
        date_index: usize,
    ) -> Option<(Symbol, f64, f64)> {
        let symbol_index = *self.index.get(symbol)?;
        let log_return = *self.returns.get(date_index)?.get(symbol_index)?;
        let state = market.underlying(symbol)?;
        Some((symbol.clone(), state.spot() * log_return.exp(), state.vol()))
    }

    /// Method 3: historical simulation with full revaluation. For each date in the
    /// return window (the most recent [`MAX_HISTORICAL_SCENARIOS`] dates at most), shocks every calibrated
    /// symbol's spot by `exp(historical log return)` on that date — vol unchanged
    /// (sticky strike) — and reprices every position with
    /// [`PositionBook::value_all`]. Each scenario's P&L is measured against an
    /// **unshocked** revaluation of the same book at the same `valuation_time` (not
    /// `summary.totals.market_value`), computed with the exact same call, so a
    /// zero-return scenario mechanically gives exactly zero P&L.
    ///
    /// 99 scenarios make the 99 % figure crude — a single order statistic near the edge
    /// of 99 points swings the whole `VaR99` estimate. Vol is held at today's level in
    /// every scenario (sticky strike), so without a history of *implied* vol this
    /// method entirely misses vega risk: an options book whose real risk is a vol
    /// collapse, not a price move, will look safer here than it is.
    fn compute_historical(
        &self,
        summary: &RiskSummary,
        market: &MarketState,
        book: &PositionBook,
        valuation_time: DateTime<Utc>,
    ) -> VarMethodReport {
        let started = Instant::now();
        // The window is the most recent scenarios: with more than the cap of dates, the
        // oldest are the ones dropped, never the newest.
        let total_dates = self.return_dates.len();
        let scenario_count = total_dates.min(MAX_HISTORICAL_SCENARIOS);
        let first_scenario = total_dates - scenario_count;

        if scenario_count == 0 {
            return VarMethodReport {
                var_95: 0.0,
                var_99: 0.0,
                es_97_5: 0.0,
                scenario_count: 0,
                window_start: None,
                window_end: None,
                compute_micros: micros_since(started),
            };
        }

        let active_value = |m: &MarketState| -> f64 {
            book.value_all(m, valuation_time)
                .iter()
                .filter(|row| row.status == PositionStatus::Active)
                .map(|row| row.market_value)
                .sum()
        };
        let base_total = active_value(market);
        let symbols_in_book: Vec<&Symbol> = summary.by_underlying.keys().collect();

        let mut losses = Vec::with_capacity(scenario_count);
        for date_index in first_scenario..total_dates {
            let mut shocked = market.clone();
            let updates: Vec<(Symbol, f64, f64)> = symbols_in_book
                .iter()
                .filter_map(|&symbol| self.shocked_update(market, symbol, date_index))
                .collect();
            // A failed update (only possible if `updates` names a symbol the shocked
            // market itself does not hold, which cannot happen since `shocked` is a
            // clone of `market` and every update comes from a symbol already looked up
            // in `market`) is ignored defensively rather than propagated: the scenario
            // then falls back to the unshocked market, contributing zero P&L rather
            // than aborting the whole computation.
            let _ = shocked.update(shocked.as_of(), updates);
            losses.push(base_total - active_value(&shocked));
        }

        losses.sort_by(f64::total_cmp);
        let var_95 = interpolated_quantile(&losses, 0.95);
        let var_99 = interpolated_quantile(&losses, 0.99);
        let threshold_97_5 = interpolated_quantile(&losses, 0.975);
        let es_97_5 = tail_mean_at_or_beyond(&losses, threshold_97_5);

        VarMethodReport {
            var_95,
            var_99,
            es_97_5,
            scenario_count,
            window_start: self.return_dates.get(first_scenario).copied(),
            window_end: self.return_dates.last().copied(),
            compute_micros: micros_since(started),
        }
    }
}

/// Microseconds elapsed since `started`, saturating at `u64::MAX` rather than panicking
/// on an implausibly long duration.
fn micros_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    // Reference values (z-scores, the ES factor) are pasted verbatim from the prompt /
    // mpmath, so keep every digit.
    #![allow(clippy::excessive_precision, clippy::unreadable_literal)]
    use approx::assert_relative_eq;
    use chrono::{NaiveDate, TimeZone};
    use domain::{Instrument, OptionContract, OptionType, Position, PositionId, UnderlyingState};
    use portfolio::PositionBook;

    use super::*;
    use crate::summary::{PortfolioTotals, UnderlyingRisk};

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 30, 14, 30, 0).unwrap()
    }

    /// A market with one entry per `(symbol, spot, vol)`, rate 4 %, no dividends.
    fn market_with(entries: &[(&str, f64, f64)]) -> MarketState {
        MarketState::new(
            now(),
            0.04,
            entries
                .iter()
                .map(|&(s, spot, vol)| (sym(s), UnderlyingState::new(spot, vol, 0.0).unwrap())),
        )
        .unwrap()
    }

    /// A `RiskSummary` with one `UnderlyingRisk` per `(symbol, dollar_delta,
    /// dollar_gamma_1pct)`, everything else default.
    fn summary_with(entries: &[(&str, f64, f64)]) -> RiskSummary {
        let by_underlying = entries
            .iter()
            .map(|&(s, dollar_delta, dollar_gamma_1pct)| {
                (
                    sym(s),
                    UnderlyingRisk {
                        dollar_delta,
                        dollar_gamma_1pct,
                        ..UnderlyingRisk::default()
                    },
                )
            })
            .collect();
        RiskSummary {
            by_underlying,
            totals: PortfolioTotals::default(),
            concentration: Vec::new(),
            active_positions: entries.len(),
            expired_positions: 0,
            error_positions: 0,
        }
    }

    fn calculator(
        symbols: &[&str],
        correlation: Vec<Vec<f64>>,
        method: VarMethod,
    ) -> VarCalculator {
        VarCalculator::new(
            symbols.iter().map(|s| sym(s)).collect(),
            correlation,
            Vec::new(),
            Vec::new(),
            method,
        )
    }

    fn empty_book() -> PositionBook {
        PositionBook::new()
    }

    // --- Test 1: delta-normal exactness for a single position -------------------------

    #[test]
    fn delta_normal_var_99_is_exact_for_a_single_underlying() {
        let market = market_with(&[("SPY", 100.0, 0.20)]);
        let summary = summary_with(&[("SPY", 5_000.0, 0.0)]);
        let mut calc = calculator(&["SPY"], vec![vec![1.0]], VarMethod::DeltaNormal);

        let report = calc.compute(&summary, &market, &empty_book(), now());

        let daily_vol = 0.20 / 252.0_f64.sqrt();
        let expected = normal::inv_cdf(0.99) * 5_000.0_f64.abs() * daily_vol;
        assert_relative_eq!(report.delta_normal.var_99, expected, max_relative = 1e-12);
        assert_eq!(report.delta_normal.scenario_count, 0);
        assert!(report.delta_normal.window_start.is_none());
    }

    // --- Test 2: correlation adds and cancels delta-normal risk ------------------------

    #[test]
    fn perfectly_correlated_same_sign_deltas_add_linearly() {
        let market = market_with(&[("AAA", 100.0, 0.20), ("BBB", 50.0, 0.30)]);
        let summary = summary_with(&[("AAA", 3_000.0, 0.0), ("BBB", 4_000.0, 0.0)]);
        let correlation = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
        let mut calc = calculator(&["AAA", "BBB"], correlation, VarMethod::DeltaNormal);

        let report = calc.compute(&summary, &market, &empty_book(), now());

        let vol_a = 0.20 / 252.0_f64.sqrt();
        let vol_b = 0.30 / 252.0_f64.sqrt();
        let sd = 3_000.0 * vol_a + 4_000.0 * vol_b;
        let expected = normal::inv_cdf(0.99) * sd;
        assert_relative_eq!(report.delta_normal.var_99, expected, max_relative = 1e-9);
    }

    #[test]
    fn perfectly_correlated_opposite_sign_equal_positions_offset_to_zero() {
        let market = market_with(&[("AAA", 100.0, 0.20), ("BBB", 100.0, 0.20)]);
        let summary = summary_with(&[("AAA", 5_000.0, 0.0), ("BBB", -5_000.0, 0.0)]);
        let correlation = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
        let mut calc = calculator(&["AAA", "BBB"], correlation, VarMethod::DeltaNormal);

        let report = calc.compute(&summary, &market, &empty_book(), now());

        assert_relative_eq!(report.delta_normal.var_99, 0.0, epsilon = 1e-6);
    }

    // --- Test 3: with zero gamma, delta-gamma MC matches delta-normal ------------------

    #[test]
    fn zero_gamma_delta_gamma_mc_matches_delta_normal_within_three_percent() {
        let market = market_with(&[("AAA", 100.0, 0.25), ("BBB", 60.0, 0.35)]);
        let summary = summary_with(&[("AAA", 8_000.0, 0.0), ("BBB", -3_000.0, 0.0)]);
        let correlation = vec![vec![1.0, 0.3], vec![0.3, 1.0]];
        let mut calc = calculator(&["AAA", "BBB"], correlation, VarMethod::DeltaGammaMc);

        let report = calc.compute(&summary, &market, &empty_book(), now());

        let relative_diff = (report.delta_gamma_mc.var_99 - report.delta_normal.var_99).abs()
            / report.delta_normal.var_99;
        assert!(
            relative_diff < 0.03,
            "delta-gamma MC {} should be within 3% of delta-normal {}, got {:.4}",
            report.delta_gamma_mc.var_99,
            report.delta_normal.var_99,
            relative_diff
        );
        assert_eq!(report.delta_gamma_mc.scenario_count, MC_SCENARIOS);
    }

    // --- Test 4: a short ATM straddle -------------------------------------------------

    /// A short `quantity` (positive number of contracts) ATM call and put on `spy`,
    /// `days` from expiry, priced with real BSM Greeks via `portfolio::PositionBook`.
    fn short_straddle_book(spy: &Symbol, spot: f64, days: i64) -> PositionBook {
        let expiry = (now() + chrono::Duration::days(days)).date_naive();
        let call = Position::new(
            PositionId::new("straddle-call").unwrap(),
            Instrument::Option(
                OptionContract::new(spy.clone(), OptionType::Call, spot, expiry).unwrap(),
            ),
            -10.0,
            5.0,
            None,
        )
        .unwrap();
        let put = Position::new(
            PositionId::new("straddle-put").unwrap(),
            Instrument::Option(
                OptionContract::new(spy.clone(), OptionType::Put, spot, expiry).unwrap(),
            ),
            -10.0,
            5.0,
            None,
        )
        .unwrap();
        PositionBook::from_positions([call, put]).unwrap()
    }

    #[test]
    fn short_atm_straddle_is_the_reason_delta_normal_is_not_enough() {
        let spy = sym("SPY");
        // Dividend yield 0.06 is chosen so r - q + ½σ² = 0.04 - 0.06 + 0.02 = 0 exactly,
        // nulling d1: the call and put deltas are then e^(-qT)·N(0) and -e^(-qT)·N(0),
        // which cancel exactly (up to floating-point noise) regardless of time to
        // expiry — an ATM straddle with genuinely (not just approximately) zero net
        // delta, isolating the point this test makes: even *exactly* zero delta hides
        // large convexity risk.
        let market = MarketState::new(
            now(),
            0.04,
            [(
                spy.clone(),
                UnderlyingState::new(100.0, 0.20, 0.06).unwrap(),
            )],
        )
        .unwrap();
        let book = short_straddle_book(&spy, 100.0, 30);
        let rows = book.value_all(&market, now());
        let summary = crate::aggregate::aggregate(&rows);

        // A tiny hand-built historical window: a straddle loses money on a move either
        // way, so a handful of symmetric shocks is enough to show a large historical
        // VaR too, without needing anything close to a real 99-point window.
        let returns = vec![
            vec![-0.03],
            vec![-0.02],
            vec![-0.01],
            vec![0.01],
            vec![0.02],
            vec![0.03],
        ];
        let return_dates: Vec<NaiveDate> = (0..6)
            .map(|d| NaiveDate::from_ymd_opt(2026, 8, 1).unwrap() + chrono::Duration::days(d))
            .collect();
        let mut calc = VarCalculator::new(
            vec![spy.clone()],
            vec![vec![1.0]],
            returns,
            return_dates,
            VarMethod::DeltaGammaMc,
        );

        let report = calc.compute(&summary, &market, &book, now());

        // A short straddle's call and put deltas roughly cancel (≈+0.5 and ≈−0.5), so
        // the linear, delta-only risk delta-normal measures is near zero — but both
        // legs' gammas are large and same-signed (a short option always has negative
        // gamma), so a move in either direction loses money fast. This is the central
        // reason delta-normal is not enough for an option book: it is blind to
        // convexity, and a book can carry a small dollar delta while carrying enormous
        // real risk.
        assert!(
            report.delta_normal.var_99 < 0.1 * report.delta_gamma_mc.var_99,
            "delta-normal {} should be tiny next to delta-gamma MC {}",
            report.delta_normal.var_99,
            report.delta_gamma_mc.var_99
        );
        assert!(
            report.delta_normal.var_99 < 0.1 * report.historical.var_99,
            "delta-normal {} should be tiny next to historical {}",
            report.delta_normal.var_99,
            report.historical.var_99
        );
        assert!(report.delta_gamma_mc.var_99 > 0.0);
        assert!(report.historical.var_99 > 0.0);
    }

    // --- Test 5: historical quantiles on a hand-built scenario set ---------------------

    #[test]
    fn interpolated_quantile_matches_hand_computed_values() {
        let losses = [
            -500.0, -200.0, -100.0, -50.0, 0.0, 50.0, 100.0, 300.0, 800.0, 1200.0,
        ];
        // i = 0.95 * 9 = 8.55: interpolate losses[8]=800 and losses[9]=1200 at 0.55.
        assert_relative_eq!(interpolated_quantile(&losses, 0.95), 1020.0, epsilon = 1e-9);
        // i = 0.99 * 9 = 8.91: interpolate losses[8]=800 and losses[9]=1200 at 0.91.
        assert_relative_eq!(interpolated_quantile(&losses, 0.99), 1164.0, epsilon = 1e-9);

        let var_99 = interpolated_quantile(&losses, 0.99);
        // i = 0.975 * 9 = 8.775: threshold = 800 + 0.775 * 400 = 1110.0. Only 1200.0 in
        // the array is >= 1110.0.
        let threshold = interpolated_quantile(&losses, 0.975);
        assert_relative_eq!(threshold, 1110.0, epsilon = 1e-9);
        let es_97_5 = tail_mean_at_or_beyond(&losses, threshold);
        assert_relative_eq!(es_97_5, 1200.0, epsilon = 1e-9);
        assert!(
            es_97_5 >= var_99,
            "ES should never be below VaR at the same level"
        );
    }

    // Exact zeros are the point: every scenario in the window is a zero return.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_longer_history_uses_the_most_recent_99_scenarios() {
        // 120 dates. The 21 oldest returns are a 40 % crash that a long stock position
        // would feel badly, and the 99 most recent are all zero. Only the recent 99 count,
        // so every VaR figure is exactly zero. Taking the first 99 would include the crash.
        let spy = sym("SPY");
        let market = market_with(&[("SPY", 100.0, 0.20)]);
        let position = Position::new(
            PositionId::new("spy-stock").unwrap(),
            Instrument::Equity {
                symbol: spy.clone(),
            },
            100.0,
            100.0,
            None,
        )
        .unwrap();
        let book = PositionBook::from_positions([position]).unwrap();
        let rows = book.value_all(&market, now());
        let summary = crate::aggregate::aggregate(&rows);
        let first_day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let dates: Vec<NaiveDate> = (0..120).map(|i| first_day + chrono::Days::new(i)).collect();
        let returns: Vec<Vec<f64>> = (0..120)
            .map(|i| vec![if i < 21 { -0.5 } else { 0.0 }])
            .collect();

        let mut calc = VarCalculator::new(
            vec![spy],
            vec![vec![1.0]],
            returns,
            dates.clone(),
            VarMethod::Historical,
        );
        let report = calc.compute(&summary, &market, &book, now()).historical;

        assert_eq!(report.scenario_count, 99);
        assert_eq!(report.window_start, Some(dates[21]));
        assert_eq!(report.window_end, Some(dates[119]));
        assert_eq!(report.var_99, 0.0);
        assert_eq!(report.es_97_5, 0.0);
    }

    #[test]
    fn a_history_within_the_cap_uses_every_date() {
        let spy = sym("SPY");
        let market = market_with(&[("SPY", 100.0, 0.20)]);
        let book = PositionBook::new();
        let summary = crate::aggregate::aggregate(&book.value_all(&market, now()));
        let day = |i| NaiveDate::from_ymd_opt(2026, 3, 1).unwrap() + chrono::Days::new(i);
        let mut calc = VarCalculator::new(
            vec![spy],
            vec![vec![1.0]],
            vec![vec![0.01], vec![-0.01], vec![0.0]],
            vec![day(0), day(1), day(2)],
            VarMethod::Historical,
        );
        let report = calc.compute(&summary, &market, &book, now()).historical;
        assert_eq!(report.scenario_count, 3);
        assert_eq!(report.window_start, Some(day(0)));
        assert_eq!(report.window_end, Some(day(2)));
    }

    // The unshocked-baseline design (see `compute_historical`'s doc comment) makes a
    // zero-return scenario's P&L exactly, not approximately, zero, so an exact
    // comparison is the correct check here.
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_zero_return_historical_scenario_gives_exactly_zero_pnl() {
        let spy = sym("SPY");
        let market = market_with(&[("SPY", 100.0, 0.20)]);
        let position = Position::new(
            PositionId::new("spy-stock").unwrap(),
            Instrument::Equity {
                symbol: spy.clone(),
            },
            100.0,
            100.0,
            None,
        )
        .unwrap();
        let book = PositionBook::from_positions([position]).unwrap();
        let rows = book.value_all(&market, now());
        let summary = crate::aggregate::aggregate(&rows);

        let mut calc = VarCalculator::new(
            vec![spy],
            vec![vec![1.0]],
            vec![vec![0.0]],
            vec![NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()],
            VarMethod::Historical,
        );
        let report = calc.compute(&summary, &market, &book, now());

        assert_eq!(report.historical.scenario_count, 1);
        assert_eq!(report.historical.var_95, 0.0);
        assert_eq!(report.historical.var_99, 0.0);
        assert_eq!(report.historical.es_97_5, 0.0);
    }

    // --- Test 6: common random numbers give identical results -------------------------

    #[test]
    fn identical_inputs_give_identical_delta_gamma_mc_results() {
        let market = market_with(&[("AAA", 100.0, 0.25), ("BBB", 60.0, 0.35)]);
        let summary = summary_with(&[("AAA", 8_000.0, 200.0), ("BBB", -3_000.0, -150.0)]);
        let correlation = vec![vec![1.0, 0.3], vec![0.3, 1.0]];
        let mut calc = calculator(&["AAA", "BBB"], correlation, VarMethod::DeltaGammaMc);
        let book = empty_book();

        let report1 = calc.compute(&summary, &market, &book, now());
        let report2 = calc.compute(&summary, &market, &book, now());

        // Without common random numbers, two independent Monte Carlo runs with
        // identical inputs would still differ by sampling noise; this only passes
        // because the epsilon draws are cached and reused, not resampled. `compute_micros`
        // legitimately differs between the two calls, so only the risk figures are
        // compared, and by exact bit pattern rather than `assert_relative_eq!`.
        assert_eq!(
            report1.delta_gamma_mc.var_95.to_bits(),
            report2.delta_gamma_mc.var_95.to_bits()
        );
        assert_eq!(
            report1.delta_gamma_mc.var_99.to_bits(),
            report2.delta_gamma_mc.var_99.to_bits()
        );
        assert_eq!(
            report1.delta_gamma_mc.es_97_5.to_bits(),
            report2.delta_gamma_mc.es_97_5.to_bits()
        );
    }

    // --- Test 7: the normal ES factor the prompt gives ---------------------------------

    #[test]
    fn normal_es_factor_at_97_5_matches_the_reference_value() {
        let factor = normal::pdf(normal::inv_cdf(0.975)) / 0.025;
        assert_relative_eq!(factor, 2.3378027922014141, max_relative = 1e-12);
    }

    // --- The partial-sort optimisation matches a full sort ------------------------------

    #[test]
    fn partial_sort_gives_the_same_result_as_a_full_sort() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let losses: Vec<f64> = (0..MC_SCENARIOS)
            .map(|_| {
                let z: f64 = rng.sample(StandardNormal);
                z * 1_000.0
            })
            .collect();

        let mut partial = losses.clone();
        let (partial_95, partial_99, partial_es) = quantiles_and_es_partial_sort(&mut partial);

        let mut full = losses;
        full.sort_by(f64::total_cmp);
        let full_95 = interpolated_quantile(&full, 0.95);
        let full_99 = interpolated_quantile(&full, 0.99);
        let full_threshold = interpolated_quantile(&full, 0.975);
        let full_es = tail_mean_at_or_beyond(&full, full_threshold);

        assert_eq!(partial_95.to_bits(), full_95.to_bits());
        assert_eq!(partial_99.to_bits(), full_99.to_bits());
        assert_eq!(partial_es.to_bits(), full_es.to_bits());
    }

    // --- The warm-up/throttle decision, with injected timings, not wall-clock ----------

    #[test]
    fn cheap_ticks_never_throttle() {
        let samples = vec![100_u64; WARM_UP_TICKS];
        assert_eq!(decide_recompute_every_ticks(&samples), 1);
    }

    #[test]
    fn a_median_at_exactly_the_budget_does_not_throttle() {
        let samples = vec![BUDGET_MICROS; WARM_UP_TICKS];
        assert_eq!(decide_recompute_every_ticks(&samples), 1);
    }

    #[test]
    fn a_slow_median_sets_n_from_the_ratio_to_budget() {
        let samples = vec![2_500_u64; WARM_UP_TICKS];
        assert_eq!(decide_recompute_every_ticks(&samples), 3); // ceil(2500 / 1000)
    }

    #[test]
    fn an_extremely_slow_median_clamps_to_ten() {
        let samples = vec![50_000_u64; WARM_UP_TICKS];
        assert_eq!(decide_recompute_every_ticks(&samples), 10);
    }

    #[test]
    fn one_early_cache_rebuild_spike_does_not_trip_the_throttle() {
        // 19 cheap ticks and one huge one-off spike (as a first-tick cache rebuild might
        // look like if it were not excluded upstream): the median, not the maximum or
        // the first sample, decides.
        let mut samples = vec![100_u64; WARM_UP_TICKS - 1];
        samples.push(5_000_000);
        assert_eq!(decide_recompute_every_ticks(&samples), 1);
    }

    #[test]
    fn median_micros_handles_even_and_odd_counts() {
        assert_eq!(median_micros(&[]), 0);
        assert_eq!(median_micros(&[10]), 10);
        assert_eq!(median_micros(&[10, 20]), 15);
        assert_eq!(median_micros(&[30, 10, 20]), 20);
        assert_eq!(median_micros(&[1, 2, 3, 4]), 2);
    }

    #[test]
    fn recompute_every_ticks_is_reported_on_var_report() {
        let market = market_with(&[("SPY", 100.0, 0.20)]);
        let summary = summary_with(&[("SPY", 5_000.0, 0.0)]);
        let mut calc = calculator(&["SPY"], vec![vec![1.0]], VarMethod::DeltaNormal);
        let report = calc.compute(&summary, &market, &empty_book(), now());
        assert_eq!(report.recompute_every_ticks, 1);
        assert!(report.freshly_computed);
    }
}
