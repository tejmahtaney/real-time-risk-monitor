//! The tick loop that joins the price feed, the book and risk. Publishes immutable
//! snapshots. No HTTP.

#![warn(missing_docs)]

mod command;
mod engine;
mod error;
mod handle;
mod recorder;
mod snapshot;

pub use command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
pub use engine::Engine;
pub use error::EngineError;
pub use handle::EngineHandle;
pub use recorder::{
    ControlAction, DailyPnlRow, PositionChangeKind, SessionEvent, SessionMeta, copy_session_config,
};
pub use snapshot::{FeedStatus, HistoryPoint, RiskSnapshot, SymbolSnapshot};

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
pub use portfolio::{
    PortfolioError, PositionRisk, PositionSpec, PositionStatus, load_csv, parse_csv, to_csv_bytes,
    to_spec,
};
pub use risk::{
    AlertEvent, Concentration, LimitKind, LimitState, LimitStatus, LimitsConfig, LimitsConfigError,
    PnlBuckets, PnlLevel, PnlReport, PortfolioLimits, PortfolioTotals, RiskSummary, Scope,
    UnderlyingDeltaLimit, UnderlyingRisk,
};
pub use storage::{
    StorageError, StorageRoot, append_jsonl, check_part, list, list_dirs, read_json, read_jsonl,
    size_of, slugify, usage_by_folder, write_bytes_new, write_json_atomic,
};
