//! Prints the calibrated seed: spot, both vols and the correlation matrix.
//!
//! Run from the repo root: `cargo run -p marketdata --example show_seed`.
//! It never calls the API. History comes from the local archive, else the fallback files.

use std::path::Path;

use marketdata::{
    AlphaVantageSource, Archive, DataMode, HistoryLoader, MarketConfig, SystemClock, calibrate,
};
use storage::StorageRoot;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = MarketConfig::load(Path::new("config/market.toml"))?;
    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(StorageRoot::resolve()?),
        "data/fallback",
        config.cache_max_age_hours,
        None,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&config.symbols, DataMode::Offline)
        .await?;
    let seed = calibrate(&histories, &config)?;

    println!(
        "as of {}, rate {:.6}, window {} returns, chosen vol: {:?}",
        seed.as_of, seed.rate, config.vol_window_days, config.vol_method
    );
    println!("symbol      spot   realised      ewma  yield  source");
    for (symbol, s) in &seed.underlyings {
        println!(
            "{:<6} {:>9.2} {:>9.2}% {:>8.2}% {:>5.2}%  {:?}",
            symbol.as_str(),
            s.spot,
            s.vol_realised * 100.0,
            s.vol_ewma * 100.0,
            s.dividend_yield * 100.0,
            s.source
        );
    }
    println!("\ncorrelation");
    print!("{:>6}", "");
    for symbol in &seed.symbols {
        print!("{:>7}", symbol.as_str());
    }
    println!();
    for (symbol, row) in seed.symbols.iter().zip(&seed.correlation) {
        print!("{:>6}", symbol.as_str());
        for value in row {
            print!("{value:>7.3}");
        }
        println!();
    }
    Ok(())
}
