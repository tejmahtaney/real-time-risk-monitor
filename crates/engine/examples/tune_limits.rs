//! Runs the simulator headless for 10 sessions over the sample portfolio with the default
//! seed, evaluates `config/limits.toml` on every tick, and prints the distribution of each
//! limited measure plus the resulting alert counts. Also grid-searches, for every
//! candidate measure, the single limit value that gives the best per-session spread of
//! `Warning`/`Breach` entries. Used to pick the values in `config/limits.toml`.
//!
//! Run from the repo root: `cargo run -p engine --example tune_limits`.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use domain::{FixedClock, SimClock, StepKind, Symbol};
use marketdata::{
    AlphaVantageSource, Archive, DataMode, GbmSimulator, HistoryLoader, MarketConfig, PriceFeed,
    SimulationConfig, SystemClock, calibrate,
};
use portfolio::{PositionBook, load_csv, resolve};
use risk::{
    AlertEvent, LimitKind, LimitMonitor, LimitState, LimitsConfig, RiskConfig, RiskSummary,
    VarCalculator, aggregate,
};
use storage::StorageRoot;

/// How many sessions to run.
const SESSIONS: usize = 10;

/// How many candidate limits [`Series::search_best_limit`] tries, log-spaced.
const SEARCH_STEPS: usize = 200;

/// Ok < Warning < Breach, so a plain comparison tells an "enter" (rising) event from a
/// "clear" (falling) one.
fn severity(state: LimitState) -> u8 {
    match state {
        LimitState::Ok => 0,
        LimitState::Warning => 1,
        LimitState::Breach => 2,
    }
}

/// Whether `event` raises a limit's severity (an "enter") or lowers it (a "clear").
fn is_enter(event: &AlertEvent) -> bool {
    severity(event.to) > severity(event.from)
}

/// A copy of `risk::monitor`'s private state-transition rule, so this offline search can
/// try many candidate limits against an already-recorded series without re-running the
/// simulator. Must be kept in step with `risk::monitor::next_state`.
fn next_state(
    current: LimitState,
    utilisation: f64,
    warning_clear_threshold: f64,
    warning_threshold: f64,
    clear_threshold: f64,
) -> LimitState {
    if current == LimitState::Breach && utilisation >= clear_threshold {
        return LimitState::Breach;
    }
    if utilisation >= 1.0 {
        return LimitState::Breach;
    }
    let entry_threshold = if current == LimitState::Ok {
        warning_threshold
    } else {
        warning_clear_threshold
    };
    if utilisation >= entry_threshold {
        LimitState::Warning
    } else {
        LimitState::Ok
    }
}

/// Every value a measure took across the run, split by session, for min/percentile/max
/// reporting and for the offline threshold search.
#[derive(Default)]
struct Series {
    by_session: Vec<Vec<f64>>,
}

impl Series {
    fn push(&mut self, session: usize, value: f64) {
        if self.by_session.len() <= session {
            self.by_session.resize_with(session + 1, Vec::new);
        }
        self.by_session[session].push(value);
    }

    fn all_values(&self) -> impl Iterator<Item = f64> + '_ {
        self.by_session.iter().flatten().copied()
    }

    /// The value at fraction `p` (0 to 1) of the sorted sample. Linear index, no interpolation.
    fn percentile(&self, p: f64) -> f64 {
        let mut sorted: Vec<f64> = self.all_values().collect();
        if sorted.is_empty() {
            return 0.0;
        }
        sorted.sort_by(f64::total_cmp);
        // p is in [0, 1] and sorted is non-empty, so the rounded index always lands in
        // [0, sorted.len() - 1].
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let index = (((sorted.len() - 1) as f64) * p).round() as usize;
        sorted[index]
    }

    /// Prints as a dollar-ish figure, no decimal places.
    fn print(&self, label: &str) {
        self.print_with(label, 12, 0);
    }

    /// Prints as a fraction, four decimal places.
    fn print_fraction(&self, label: &str) {
        self.print_with(label, 10, 4);
    }

    fn print_with(&self, label: &str, width: usize, precision: usize) {
        let Some(min) = self.all_values().reduce(f64::min) else {
            println!("{label}: no data");
            return;
        };
        let max = self.all_values().reduce(f64::max).unwrap_or(0.0);
        println!(
            "{label:<32} min {min:>width$.precision$}  p50 {:>width$.precision$}  p90 {:>width$.precision$}  p99 {:>width$.precision$}  max {max:>width$.precision$}",
            self.percentile(0.50),
            self.percentile(0.90),
            self.percentile(0.99),
        );
    }

    /// Replays the state machine over the whole (continuous, not reset per session)
    /// series for one candidate `limit`, returning (enters, clears) per session.
    fn replay(&self, limit: f64, wc: f64, w: f64, c: f64) -> Vec<(u32, u32)> {
        let mut state = LimitState::Ok;
        let mut counts = vec![(0u32, 0u32); self.by_session.len()];
        for (session, values) in self.by_session.iter().enumerate() {
            for &value in values {
                let utilisation = value.abs() / limit;
                let next = next_state(state, utilisation, wc, w, c);
                if next != state {
                    if severity(next) > severity(state) {
                        counts[session].0 += 1;
                    } else {
                        counts[session].1 += 1;
                    }
                }
                state = next;
            }
        }
        counts
    }

    /// Grid-searches candidate limits between the series' own p10 and 1.5x its max,
    /// scoring each by how many sessions land with 1-3 enters (a bonus for every such
    /// session) and whether at least one clear happens anywhere (a smaller bonus, since a
    /// permanently one-way limit is otherwise a valid but less demo-friendly choice).
    /// Prints the best candidate found.
    fn search_best_limit(&self, label: &str, wc: f64, w: f64, c: f64) {
        let low = self.percentile(0.10).max(1.0);
        let high = (self.all_values().reduce(f64::max).unwrap_or(1.0) * 1.5).max(low * 1.01);
        let mut best_score = i32::MIN;
        let mut best_limit = low;
        let mut best_counts = Vec::new();
        for step in 0..=SEARCH_STEPS {
            #[allow(clippy::cast_precision_loss)]
            let t = step as f64 / SEARCH_STEPS as f64;
            let limit = low * (high / low).powf(t);
            let counts = self.replay(limit, wc, w, c);
            let sessions_in_range = counts
                .iter()
                .filter(|(enters, _)| (1..=3).contains(enters))
                .count();
            let any_clear = counts.iter().any(|(_, clears)| *clears > 0);
            #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
            let score = sessions_in_range as i32 * 10 + i32::from(any_clear);
            if score > best_score {
                best_score = score;
                best_limit = limit;
                best_counts = counts;
            }
        }
        let sessions_in_range = best_counts
            .iter()
            .filter(|(enters, _)| (1..=3).contains(enters))
            .count();
        println!(
            "{label:<32} best limit ~{best_limit:>10.0}  sessions in [1,3]: {sessions_in_range}/{}  counts: {best_counts:?}",
            best_counts.len(),
        );
    }
}

/// Every candidate measure's recorded series, gathered as the simulation runs.
#[derive(Default)]
struct Measures {
    dollar_delta: Series,
    dollar_gamma_1pct: Series,
    vega: Series,
    theta_decay: Series,
    gross_exposure: Series,
    concentration: Series,
    /// The headline `VaR` method's `VaR99` (Part 18), recorded from a real `VarCalculator`
    /// run alongside every other measure — not a placeholder — so `var_99` in
    /// `config/limits.toml` can be tuned the same way every other limit already is.
    var_99: Series,
    underlying_dollar_delta: BTreeMap<Symbol, Series>,
}

impl Measures {
    fn record(&mut self, session: usize, summary: &RiskSummary, headline_var_99: f64) {
        self.dollar_delta
            .push(session, summary.totals.dollar_delta.abs());
        self.dollar_gamma_1pct
            .push(session, summary.totals.dollar_gamma_1pct.abs());
        self.vega.push(session, summary.totals.vega.abs());
        self.theta_decay
            .push(session, (-summary.totals.theta).max(0.0));
        self.gross_exposure
            .push(session, summary.totals.gross_exposure);
        self.concentration.push(
            session,
            summary.concentration.first().map_or(0.0, |c| c.fraction),
        );
        self.var_99.push(session, headline_var_99);
        for (symbol, risk) in &summary.by_underlying {
            self.underlying_dollar_delta
                .entry(symbol.clone())
                .or_default()
                .push(session, risk.dollar_delta.abs());
        }
    }

    fn print_distributions(&self) {
        println!("Distribution of each limited measure over {SESSIONS} sessions:");
        self.dollar_delta.print("portfolio |dollar delta|");
        self.dollar_gamma_1pct.print("portfolio |dollar gamma 1%|");
        self.vega.print("portfolio |vega|");
        self.theta_decay
            .print("portfolio theta decay (max(0,-theta))");
        self.gross_exposure.print("gross exposure");
        self.concentration
            .print_fraction("largest concentration share");
        self.var_99.print("headline VaR99 (Part 18)");
        for (symbol, series) in &self.underlying_dollar_delta {
            series.print(&format!("{symbol} |dollar delta|"));
        }
    }

    fn print_search(&self, wc: f64, w: f64, c: f64) {
        println!(
            "\nBest single-limit candidate per measure (searched against warning_clear_threshold={wc}, warning_threshold={w}, clear_threshold={c}):"
        );
        self.dollar_delta
            .search_best_limit("portfolio |dollar delta|", wc, w, c);
        self.dollar_gamma_1pct
            .search_best_limit("portfolio |dollar gamma 1%|", wc, w, c);
        self.vega.search_best_limit("portfolio |vega|", wc, w, c);
        self.theta_decay
            .search_best_limit("portfolio theta decay", wc, w, c);
        self.gross_exposure
            .search_best_limit("gross exposure", wc, w, c);
        self.concentration
            .search_best_limit("largest concentration share", wc, w, c);
        for (symbol, series) in &self.underlying_dollar_delta {
            series.search_best_limit(&format!("{symbol} |dollar delta|"), wc, w, c);
        }
    }
}

/// Prints the alert summary: per-session enters/clears, totals by limit, and when the
/// first alert landed relative to a default-speed run's real clock.
fn print_alert_summary(
    session_counts: &[(u32, u32)],
    alerts_by_kind: &BTreeMap<LimitKind, u32>,
    first_alert_at: Option<DateTime<Utc>>,
    start: DateTime<Utc>,
    speed: f64,
) {
    println!(
        "\nUnder config/limits.toml, per-session (enters, clears) into/out of Warning or Breach: {session_counts:?}"
    );
    println!("By limit: {alerts_by_kind:?}");
    match first_alert_at {
        Some(at) => {
            let sim_elapsed = at - start;
            let real_seconds = sim_elapsed.num_milliseconds() as f64 / 1000.0 / speed;
            println!(
                "First alert at simulated {sim_elapsed}, {real_seconds:.1} real seconds into a default-speed run"
            );
        }
        None => println!("No alerts raised in {SESSIONS} sessions"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let market = MarketConfig::load(Path::new("config/market.toml"))?;
    let sim_config = SimulationConfig::load(Path::new("config/simulation.toml"))?;
    let limits = LimitsConfig::load(Path::new("config/limits.toml"))?;
    let risk_config = RiskConfig::load(Path::new("config/risk.toml"))?;
    let (wc, w, c) = (
        limits.warning_clear_threshold,
        limits.warning_threshold,
        limits.clear_threshold,
    );

    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(StorageRoot::resolve()?),
        "data/fallback",
        market.cache_max_age_hours,
        None,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&market.symbols, DataMode::Offline)
        .await?;
    let seed = calibrate(&histories, &market)?;

    let start = sim_config.start_time(&seed);
    let mut clock = SimClock::new(start, sim_config.speed)?;
    let mut feed = GbmSimulator::new(&seed, &sim_config)?;
    let mut market_state = seed.to_market_state(start)?;

    let specs = load_csv(Path::new("data/sample_portfolio.csv"))?;
    let positions = resolve(&specs, &market_state, start)?;
    let mut book = PositionBook::from_positions(positions)?;

    let mut monitor = LimitMonitor::new(limits, Box::new(FixedClock::new(start)));
    let mut var_calculator = VarCalculator::new(
        seed.symbols.clone(),
        seed.correlation.clone(),
        seed.returns.clone(),
        seed.return_dates.clone(),
        risk_config.headline_var_method,
    );

    let mut measures = Measures::default();
    let mut session_counts: Vec<(u32, u32)> = Vec::new();
    let mut enters_this_session = 0u32;
    let mut clears_this_session = 0u32;
    let mut alerts_by_kind: BTreeMap<LimitKind, u32> = BTreeMap::new();
    let mut first_alert_at: Option<DateTime<Utc>> = None;
    let mut session = 0usize;

    loop {
        let step = clock.advance(sim_config.tick_interval());
        let tick = feed.step(&step)?;
        tick.apply_to(&mut market_state)?;
        book.settle_expired(step.to, &market_state);
        let rows = book.value_all(&market_state, step.to);
        let summary = aggregate(&rows);
        let var_report = var_calculator.compute(&summary, &market_state, &book, step.to);
        let headline_var_99 = var_report.headline().var_99;
        measures.record(session, &summary, headline_var_99);

        let events = monitor.evaluate(&summary, headline_var_99, step.to);
        for event in &events {
            *alerts_by_kind.entry(event.name).or_default() += 1;
            if is_enter(event) {
                enters_this_session += 1;
            } else {
                clears_this_session += 1;
            }
        }
        if first_alert_at.is_none() && events.iter().any(is_enter) {
            first_alert_at = Some(step.to);
        }

        if step.kind == StepKind::Overnight {
            session_counts.push((enters_this_session, clears_this_session));
            enters_this_session = 0;
            clears_this_session = 0;
            session += 1;
            if session >= SESSIONS {
                break;
            }
        }
    }

    measures.print_distributions();
    print_alert_summary(
        &session_counts,
        &alerts_by_kind,
        first_alert_at,
        start,
        sim_config.speed,
    );
    measures.print_search(wc, w, c);

    Ok(())
}
