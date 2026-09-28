//! The tick loop that joins the price feed, the book and risk. Publishes immutable
//! snapshots. No HTTP.

#![warn(missing_docs)]

mod command;
mod engine;
mod error;
mod handle;
mod snapshot;

pub use command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
pub use engine::Engine;
pub use error::EngineError;
pub use handle::EngineHandle;
pub use snapshot::{FeedStatus, RiskSnapshot, SymbolSnapshot};

// Re-exported so `server` can name every type it needs without adding these crates as
// direct dependencies of its own, keeping the dependency direction in CLAUDE.md intact
// (`server` depends on `engine` only).
pub use domain::{
    Clock, DomainError, FixedClock, InstrumentKind, Position, PositionId, SimClock, Symbol,
};
pub use marketdata::{
    AlphaVantageSource, Archive, CalibrationError, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL,
    DataMode, DataSource, FeedError, GbmSimulator, HistoryLoader, MarketConfig, MarketDataError,
    PriceFeed, SeedData, SimulationConfig, SymbolSeed, SystemClock, VolProcessConfig, calibrate,
};
pub use portfolio::{PortfolioError, PositionRisk, PositionSpec, PositionStatus, load_csv};
pub use risk::{
    AlertEvent, Concentration, LimitKind, LimitState, LimitStatus, LimitsConfig, LimitsConfigError,
    PnlReport, PortfolioTotals, RiskSummary, Scope, UnderlyingRisk,
};
pub use storage::{StorageError, StorageRoot};
