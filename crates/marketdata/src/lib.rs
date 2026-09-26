//! Real market data fetch, the local archive of every download, calibration, the simulated price feed and the market clock.
//!
//! Part 7 covers the real-data half: [`AlphaVantageSource`] fetches daily closes,
//! [`Archive`] keeps every download, [`HistoryLoader`] walks the archive, API and
//! fallback chain, and [`calibrate`] turns the histories into a [`SeedData`] for the
//! simulator.

#![warn(missing_docs)]

mod archive;
mod calibration;
mod clock;
mod config;
mod error;
mod fallback;
mod loader;
mod seed;
mod source;

pub use archive::{Archive, ArchiveHit, ArchiveRecord};
pub use calibration::{EWMA_LAMBDA, TRADING_DAYS_PER_YEAR, calibrate};
pub use clock::SystemClock;
pub use config::{ConfigError, DataMode, MarketConfig, VolMethod};
pub use error::{CalibrationError, MarketDataError};
pub use fallback::{fallback_path, read_fallback, write_fallback};
pub use loader::{DataSource, HistoryLoader, LoadedHistory};
pub use seed::{SeedData, SymbolSeed};
pub use source::{
    AlphaVantageSource, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL, DailyBar, Download,
    HistorySource, RequestParams, parse_daily_body,
};
