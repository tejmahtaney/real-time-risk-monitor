//! The committed `config/market.toml`, `config/simulation.toml` and `data/fallback/*.csv` load,
//! calibrate and drive a simulated session.

use std::path::PathBuf;

use domain::{Clock, SimClock, StepKind};
use marketdata::{
    AlphaVantageSource, Archive, DataMode, DataSource, GbmSimulator, HistoryLoader, MarketConfig,
    PriceFeed, SimulationConfig, SystemClock, calibrate,
};
use storage::StorageRoot;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

#[tokio::test]
async fn committed_fallback_files_calibrate() {
    let config = MarketConfig::load(&repo_file("config/market.toml")).unwrap();
    assert_eq!(config.symbols.len(), 5);

    // An empty temp archive and no API source, so every symbol falls back to its CSV.
    let dir = tempfile::tempdir().unwrap();
    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(StorageRoot::at(dir.path())),
        repo_file("data/fallback"),
        config.cache_max_age_hours,
        None,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&config.symbols, DataMode::Offline)
        .await
        .unwrap();
    for symbol in &config.symbols {
        let loaded = &histories[symbol];
        assert!(matches!(loaded.source, DataSource::Fallback { .. }));
        assert!(loaded.bars.len() > config.vol_window_days, "{symbol}");
    }

    let seed = calibrate(&histories, &config).unwrap();
    assert_eq!(seed.symbols, config.symbols);
    assert_eq!(seed.returns.len(), config.vol_window_days);
    for (symbol, s) in &seed.underlyings {
        assert!(s.spot > 0.0, "{symbol}");
        // Equity vols sit between a few percent and a few hundred percent.
        assert!(
            (0.05..2.0).contains(&s.vol_realised),
            "{symbol}: {}",
            s.vol_realised
        );
        assert!(
            (0.05..2.0).contains(&s.vol_ewma),
            "{symbol}: {}",
            s.vol_ewma
        );
    }
    let n = config.symbols.len();
    assert_eq!(seed.correlation.len(), n);
    for (i, row) in seed.correlation.iter().enumerate() {
        assert_eq!(row.len(), n);
        assert_eq!(row[i].to_bits(), 1.0_f64.to_bits());
    }
    let market = seed.to_market_state(chrono::Utc::now()).unwrap();
    assert_eq!(market.symbols().count(), n);
}

#[test]
fn committed_simulation_config_loads() {
    let config = SimulationConfig::load(&repo_file("config/simulation.toml")).unwrap();
    assert_eq!(config.seed, 42);
    assert_eq!(config.tick_interval_ms, 250);
    assert_eq!(config.start_date, None);
}

/// Seed data from the committed files drives a full simulated session and the jump to the next.
#[tokio::test]
async fn committed_seed_drives_a_full_session() {
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
    assert_eq!(clock.now(), start, "the default start is a session open");
    let mut feed = GbmSimulator::new(&seed, &sim_config).unwrap();
    let mut market_state = seed.to_market_state(start).unwrap();

    let mut steps = 0_u32;
    let last = loop {
        let step = clock.advance(sim_config.tick_interval());
        let tick = feed.step(&step).unwrap();
        tick.apply_to(&mut market_state).unwrap();
        for (symbol, quote) in &tick.quotes {
            assert!(quote.spot.is_finite() && quote.spot > 0.0, "{symbol}");
            assert!(quote.vol.is_finite() && quote.vol > 0.0, "{symbol}");
        }
        steps += 1;
        if step.kind == StepKind::Close {
            break step;
        }
        assert!(steps < 10_000, "the session never closed");
    };
    // 6.5 hours at 120x with 250 ms ticks is 780 steps, the last ending at the close.
    assert_eq!(steps, 780);
    assert_eq!(last.to, clock.now());
    assert_eq!(market_state.as_of(), last.to);
    assert_eq!(market_state.symbols().count(), market.symbols.len());

    let overnight = clock.advance(sim_config.tick_interval());
    assert_eq!(overnight.kind, StepKind::Overnight);
    let before = feed.current_tick(last.to, StepKind::Close);
    assert_eq!(feed.step(&overnight).unwrap().quotes, before.quotes);
}
