//! The committed `config/market.toml` and `data/fallback/*.csv` load and calibrate.

use std::path::PathBuf;

use marketdata::{
    AlphaVantageSource, Archive, DataMode, DataSource, HistoryLoader, MarketConfig, SystemClock,
    calibrate,
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
