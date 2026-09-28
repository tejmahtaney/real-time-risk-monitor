//! Runs one full simulated session over the sample portfolio with the committed fallback data
//! and a fixed seed, and checks that P&L attribution explains almost all of the day's move.

use std::path::PathBuf;

use domain::{ClockStep, SimClock, StepKind, market_date};
use marketdata::{
    AlphaVantageSource, Archive, DataMode, GbmSimulator, HistoryLoader, MarketConfig, PriceFeed,
    SimulationConfig, SystemClock, calibrate,
};
use portfolio::{PositionBook, PositionStatus, load_csv, resolve};
use risk::{PnlAttributor, PnlBuckets};
use storage::StorageRoot;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

/// |residual| as a share of the sum of |bucket| over the six explained buckets. Zero when
/// there is nothing to explain, rather than dividing by zero.
fn residual_share(buckets: &PnlBuckets) -> f64 {
    let denominator = buckets.delta.abs()
        + buckets.gamma.abs()
        + buckets.vega.abs()
        + buckets.theta.abs()
        + buckets.rho.abs()
        + buckets.higher_order.abs();
    if denominator > 0.0 {
        buckets.residual.abs() / denominator
    } else {
        0.0
    }
}

#[tokio::test]
async fn attribution_over_a_full_session_stays_within_five_percent_residual() {
    let market = MarketConfig::load(&repo_file("config/market.toml")).unwrap();
    let sim_config = SimulationConfig::load(&repo_file("config/simulation.toml")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(StorageRoot::at(dir.path())),
        repo_file("data/fallback"),
        market.cache_max_age_hours,
        None,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&market.symbols, DataMode::Offline)
        .await
        .unwrap();
    let seed = calibrate(&histories, &market).unwrap();

    let start = sim_config.start_time(&seed);
    let mut clock = SimClock::new(start, sim_config.speed).unwrap();
    let mut feed = GbmSimulator::new(&seed, &sim_config).unwrap();
    let mut market_state = seed.to_market_state(start).unwrap();

    let portfolio_path = repo_file("data/sample_portfolio.csv");
    let specs = load_csv(&portfolio_path).unwrap();
    let positions = resolve(&specs, &market_state, start).unwrap();
    let mut book = PositionBook::from_positions(positions).unwrap();

    let mut attributor = PnlAttributor::new(market_date(start));
    // Seed the attributor with the session's opening valuation, a zero-length step, so the
    // first real tick has something to diff against instead of being treated as a fresh add.
    let opening_step = ClockStep {
        from: start,
        to: start,
        trading_years: 0.0,
        calendar_years: 0.0,
        session_date: market_date(start),
        kind: StepKind::Intraday,
    };
    attributor.on_tick(&book.value_all(&market_state, start), &opening_step);

    let mut steps = 0_u32;
    let mut last_rows;
    loop {
        let step = clock.advance(sim_config.tick_interval());
        let tick = feed.step(&step).unwrap();
        tick.apply_to(&mut market_state).unwrap();
        book.settle_expired(step.to, &market_state);
        let rows = book.value_all(&market_state, step.to);
        attributor.on_tick(&rows, &step);
        last_rows = rows;

        steps += 1;
        assert!(steps < 10_000, "the session never closed and rolled");
        if step.kind == StepKind::Overnight {
            break;
        }
    }

    let expiring_call_status = last_rows
        .iter()
        .find(|row| row.id.as_str() == "spy-call-expiring")
        .map(|row| row.status.clone())
        .expect("spy-call-expiring should still be in the book");
    assert_eq!(
        expiring_call_status,
        PositionStatus::Expired,
        "spy-call-expiring should have settled by the end of the session"
    );

    let report = attributor.report();
    assert!(
        report.previous_day.is_some(),
        "previous_day should be populated after the overnight roll"
    );

    let mut excluding_expiring_call = PnlBuckets::default();
    println!("per-position residual share (since start):");
    for (id, level) in &report.by_position {
        let share = residual_share(&level.since_start);
        println!("  {}: {:.4}", id.as_str(), share);
        if id.as_str() != "spy-call-expiring" {
            excluding_expiring_call += level.since_start;
        }
    }

    let portfolio_share = residual_share(&report.portfolio.since_start);
    let share_excluding_expiring_call = residual_share(&excluding_expiring_call);
    println!("portfolio residual share (since start): {portfolio_share:.4}");
    println!(
        "portfolio residual share excluding spy-call-expiring: {share_excluding_expiring_call:.4}"
    );

    assert!(
        portfolio_share < 0.05,
        "portfolio residual share {portfolio_share} is 5% or more; see the per-position shares above"
    );
}
