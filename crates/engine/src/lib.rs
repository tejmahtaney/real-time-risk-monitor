//! The tick loop that joins the price feed, the book and risk. Publishes immutable
//! snapshots. No HTTP.

#![warn(missing_docs)]

mod backtest;
mod command;
mod corporate;
mod engine;
mod error;
mod handle;
mod recorder;
mod snapshot;

pub use backtest::{
    BACKTESTS_FOLDER, BacktestConfig, BacktestError, BacktestEvent, BacktestListItem,
    BacktestManager, BacktestResults, BacktestRollEvent, BacktestRunState, BacktestSettings,
    BacktestSource, BacktestSplitHandling, BacktestSplitNote, BacktestStatus, BacktestSubscription,
    ClientGuard, ControlCommand, CreateOptions, DAILY_FILE, DEFAULT_SPEED, EVENTS_FILE, INPUT_FILE,
    MAX_SPEED, MIN_SPEED, RollPolicy, RunInput, SETTINGS_FILE, SUMMARY_FILE, SpecRoller,
};
pub use command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
pub use corporate::{
    CorporateActionEvent, CorporateActionOutcome, CorporateActionState, archive_ledger, read_ledger,
};
pub use engine::Engine;
pub use error::EngineError;
pub use handle::EngineHandle;
pub use recorder::{
    ControlAction, DailyPnlRow, LEDGER_FILE, PositionChangeKind, SessionEvent, SessionMeta,
    copy_session_config,
};
pub use snapshot::{FeedStatus, HistoryPoint, RiskSnapshot, SymbolSnapshot};

// Re-exported so `server` can name every type it needs without adding these crates as
// direct dependencies of its own, keeping the dependency direction in CLAUDE.md intact
// (`server` depends on `engine` only).
pub use domain::{
    AppliedAction, Clock, CorporateAction, DomainError, FixedClock, InstrumentKind, Position,
    PositionId, SimClock, StepKind, Symbol, market_date,
};
pub use marketdata::{
    AlphaVantageSource, Archive, CalibrationError, CorporateActionCalendar, CorporateActionError,
    CorporateActionsConfig, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL, DataMode, DataSource,
    FeedError, GbmSimulator, HistoryLoader, MarketConfig, MarketDataError, PriceFeed,
    ScheduledAction, SeedData, SimulationConfig, SymbolSeed, SystemClock, VolProcessConfig,
    apply_ledger_to_seed, calibrate, load_upcoming_dividends,
};
pub use portfolio::{
    PortfolioError, PositionRisk, PositionSpec, PositionStatus, load_csv, parse_csv, to_csv_bytes,
    to_spec,
};
pub use risk::{
    AlertEvent, BacktestSummary, Concentration, DailyRow, DayPnl, Drawdown, KupiecTest, LimitKind,
    LimitState, LimitStatus, LimitsConfig, LimitsConfigError, PnlBuckets, PnlLevel, PnlReport,
    PortfolioLimits, PortfolioTotals, RiskConfig, RiskConfigError, RiskSummary, Scope,
    UnderlyingDeltaLimit, UnderlyingRisk, VarCalculator, VarMethod, VarMethodReport, VarReport,
};
pub use storage::{
    StorageError, StorageRoot, append_jsonl, check_part, list, list_dirs, read_json, read_jsonl,
    size_of, slugify, usage_by_folder, write_bytes_new, write_json_atomic,
};
