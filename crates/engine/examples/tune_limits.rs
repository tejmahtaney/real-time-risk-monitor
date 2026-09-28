//! Runs the simulator headless for 10 sessions over the sample portfolio with the default
//! seed, evaluates `config/limits.toml` on every tick, and prints the distribution of each
//! limited measure plus the resulting alert counts. Used to pick limit values that show a
//! handful of alerts per demo session.
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
use risk::{LimitKind, LimitMonitor, LimitsConfig, aggregate};
use storage::StorageRoot;

/// How many sessions to run.
const SESSIONS: u32 = 10;

/// Every value a measure took across the run, for min/percentile/max reporting.
#[derive(Default)]
struct Distribution {
    values: Vec<f64>,
}

impl Distribution {
    fn push(&mut self, value: f64) {
        self.values.push(value);
    }

    /// The value at fraction `p` (0 to 1) of the sorted sample. Linear index, no interpolation.
    fn percentile(&self, p: f64) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let mut sorted = self.values.clone();
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
        let Some(min) = self.values.iter().copied().reduce(f64::min) else {
            println!("{label}: no data");
            return;
        };
        let max = self.values.iter().copied().reduce(f64::max).unwrap_or(0.0);
        println!(
            "{label:<32} min {min:>width$.precision$}  p50 {:>width$.precision$}  p90 {:>width$.precision$}  p99 {:>width$.precision$}  max {max:>width$.precision$}",
            self.percentile(0.50),
            self.percentile(0.90),
            self.percentile(0.99),
        );
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let market = MarketConfig::load(Path::new("config/market.toml"))?;
    let sim_config = SimulationConfig::load(Path::new("config/simulation.toml"))?;
    let limits = LimitsConfig::load(Path::new("config/limits.toml"))?;

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

    let mut dollar_delta = Distribution::default();
    let mut dollar_gamma_1pct = Distribution::default();
    let mut vega = Distribution::default();
    let mut theta_decay = Distribution::default();
    let mut gross_exposure = Distribution::default();
    let mut concentration = Distribution::default();
    let mut underlying_dollar_delta: BTreeMap<Symbol, Distribution> = BTreeMap::new();

    let mut session_alerts = Vec::new();
    let mut alerts_this_session = 0u32;
    let mut alerts_by_kind: BTreeMap<LimitKind, u32> = BTreeMap::new();
    let mut first_alert_at: Option<DateTime<Utc>> = None;
    let mut sessions_done = 0u32;

    loop {
        let step = clock.advance(sim_config.tick_interval());
        let tick = feed.step(&step)?;
        tick.apply_to(&mut market_state)?;
        book.settle_expired(step.to, &market_state);
        let rows = book.value_all(&market_state, step.to);
        let summary = aggregate(&rows);

        dollar_delta.push(summary.totals.dollar_delta.abs());
        dollar_gamma_1pct.push(summary.totals.dollar_gamma_1pct.abs());
        vega.push(summary.totals.vega.abs());
        theta_decay.push((-summary.totals.theta).max(0.0));
        gross_exposure.push(summary.totals.gross_exposure);
        concentration.push(summary.concentration.first().map_or(0.0, |c| c.fraction));
        for (symbol, risk) in &summary.by_underlying {
            underlying_dollar_delta
                .entry(symbol.clone())
                .or_default()
                .push(risk.dollar_delta.abs());
        }

        let events = monitor.evaluate(&summary, step.to);
        alerts_this_session += u32::try_from(events.len()).unwrap_or(u32::MAX);
        for event in &events {
            *alerts_by_kind.entry(event.name).or_default() += 1;
        }
        if first_alert_at.is_none() && !events.is_empty() {
            first_alert_at = Some(step.to);
        }

        if step.kind == StepKind::Overnight {
            session_alerts.push(alerts_this_session);
            alerts_this_session = 0;
            sessions_done += 1;
            if sessions_done >= SESSIONS {
                break;
            }
        }
    }

    println!("Distribution of each limited measure over {SESSIONS} sessions:");
    dollar_delta.print("portfolio |dollar delta|");
    dollar_gamma_1pct.print("portfolio |dollar gamma 1%|");
    vega.print("portfolio |vega|");
    theta_decay.print("portfolio theta decay (max(0,-theta))");
    gross_exposure.print("gross exposure");
    concentration.print_fraction("largest concentration share");
    for (symbol, dist) in &underlying_dollar_delta {
        dist.print(&format!("{symbol} |dollar delta|"));
    }

    println!("\nWarnings/breaches entered per session: {session_alerts:?}");
    println!("By limit: {alerts_by_kind:?}");
    match first_alert_at {
        Some(at) => {
            let sim_elapsed = at - start;
            let real_seconds = sim_elapsed.num_milliseconds() as f64 / 1000.0 / sim_config.speed;
            println!(
                "First alert at simulated {sim_elapsed}, {real_seconds:.1} real seconds into a default-speed run"
            );
        }
        None => println!("No alerts raised in {SESSIONS} sessions"),
    }

    Ok(())
}
