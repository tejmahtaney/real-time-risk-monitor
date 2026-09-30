//! Errors creating, controlling and reading backtest runs.

use marketdata::{CorporateActionError, MarketDataError, ReplayDataError};
use portfolio::PortfolioError;
use storage::StorageError;
use thiserror::Error;

use crate::error::EngineError;

/// Everything that can go wrong with a backtest run.
#[derive(Debug, Error)]
pub enum BacktestError {
    /// The request itself was wrong: a bad speed, a confirmation that does not match, a finished
    /// run sent a play command.
    #[error("{0}")]
    Invalid(String),
    /// No such run.
    #[error("{0}")]
    NotFound(String),
    /// Too many runs are already in memory and unfinished.
    #[error(
        "{running} backtests are already running and the limit is {limit}: finish or stop one first"
    )]
    TooManyRunning {
        /// Runs in memory and unfinished.
        running: usize,
        /// The limit.
        limit: usize,
    },
    /// The price data could not be used.
    #[error(transparent)]
    Data(#[from] ReplayDataError),
    /// The local archive could not be read.
    #[error(transparent)]
    Archive(#[from] MarketDataError),
    /// The corporate-action config could not be resolved.
    #[error(transparent)]
    Actions(#[from] CorporateActionError),
    /// The sample portfolio could not be loaded or resolved.
    #[error(transparent)]
    Portfolio(#[from] PortfolioError),
    /// The engine could not be built.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// A file could not be read or written.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Something that should not happen, such as a replay task that ended unexpectedly.
    #[error("{0}")]
    Internal(String),
}
