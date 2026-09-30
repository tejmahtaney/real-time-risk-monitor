//! Writes `data/fallback/{SYMBOL}.csv` from real Alpha Vantage closes.
//!
//! Run from the repo root: `cargo run -p marketdata --example fetch_fallback`.
//! The key comes from `ALPHA_VANTAGE_API_KEY`, in the environment or in `.env`.
//! Every symbol goes through the archive path, so a download younger than
//! `cache_max_age_hours` is reused and the free tier's 25 requests a day are not spent
//! twice. The raw responses are archived under the storage folder, outside git.

use std::path::Path;

use marketdata::{
    AlphaVantageSource, Archive, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL, DataMode, DataSource,
    HistoryLoader, MarketConfig, SystemClock, write_fallback,
};
use storage::StorageRoot;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    // A missing .env file is fine: the variable may be set in the environment.
    let _ = dotenvy::dotenv();
    let key = std::env::var("ALPHA_VANTAGE_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or("ALPHA_VANTAGE_API_KEY is not set in the environment or in .env")?;

    let config = MarketConfig::load(Path::new("config/market.toml"))?;
    let root = StorageRoot::resolve()?;
    println!("storage folder: {}", root.path().display());
    let source = AlphaVantageSource::new(key, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL)?;
    let fallback_dir = Path::new("data/fallback");

    // No fallback directory is passed to the loader: this run must not fall back to the
    // files it is about to write.
    let loader = HistoryLoader::new(
        Archive::new(root),
        fallback_dir.join("does-not-exist-on-purpose"),
        config.cache_max_age_hours,
        Some(source),
        Box::new(SystemClock),
    );
    let histories = loader.load_history(&config.symbols, DataMode::Auto).await?;
    for (symbol, loaded) in &histories {
        let origin = match &loaded.source {
            DataSource::Api { .. } => "downloaded now",
            DataSource::Archive { .. } => "reused a fresh archive file",
            DataSource::StaleArchive { .. } => "reused a stale archive file (API failed)",
            DataSource::Fallback { .. } | DataSource::Replay { .. } => {
                return Err("unexpected fallback source".into());
            }
        };
        let path = write_fallback(fallback_dir, symbol, &loaded.bars)?;
        println!(
            "{symbol}: {} bars, {origin}, wrote {}",
            loaded.bars.len(),
            path.display()
        );
    }
    Ok(())
}
