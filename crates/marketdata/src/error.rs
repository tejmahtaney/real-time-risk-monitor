//! Error types for fetching, archiving and calibrating market data.

use chrono::NaiveDate;
use domain::{DomainError, Symbol};
use thiserror::Error;

/// Fetching, archiving or loading price history failed.
#[derive(Debug, Error)]
pub enum MarketDataError {
    /// Alpha Vantage refused the request because of a rate or daily limit
    /// (a `Note` body, or an `Information` body that talks about request limits).
    #[error("Alpha Vantage rate limit: {0}")]
    RateLimited(String),
    /// Alpha Vantage sent an `Information` body that is not about limits, such as a
    /// premium-only notice.
    #[error("Alpha Vantage information: {0}")]
    Information(String),
    /// Alpha Vantage sent an `Error Message` body, which it uses for unknown symbols
    /// and malformed requests.
    #[error("Alpha Vantage rejected the request: {0}")]
    InvalidSymbol(String),
    /// The server answered with a non-success HTTP status.
    #[error("Alpha Vantage answered HTTP {0}")]
    HttpStatus(u16),
    /// The request never completed: DNS, connection, TLS or timeout. The message never
    /// contains the request URL, so it cannot leak the API key.
    #[error("request to Alpha Vantage failed: {0}")]
    Transport(String),
    /// The body was not JSON, or lacked the daily series.
    #[error("unusable Alpha Vantage body: {0}")]
    BadBody(String),
    /// A date or close in the data was invalid.
    #[error("invalid price data: {0}")]
    InvalidData(String),
    /// A fallback CSV could not be read or written.
    #[error("fallback file {path}: {message}")]
    Fallback {
        /// The file involved.
        path: String,
        /// What went wrong.
        message: String,
    },
    /// Local storage failed.
    #[error(transparent)]
    Storage(#[from] storage::StorageError),
    /// No source produced any history for the symbol.
    #[error("no price history for {0}: no archive, API result or fallback file")]
    NoHistory(Symbol),
}

/// Calibration could not produce a seed.
#[derive(Debug, Error, PartialEq)]
pub enum CalibrationError {
    /// No histories were supplied.
    #[error("no histories to calibrate")]
    NoHistories,
    /// A configured symbol has no history.
    #[error("no history for configured symbol {0}")]
    MissingHistory(Symbol),
    /// The same date appears twice for one symbol.
    #[error("{symbol} has two closes for {date}")]
    DuplicateDate {
        /// The symbol.
        symbol: Symbol,
        /// The repeated date.
        date: NaiveDate,
    },
    /// A close is not a positive finite number.
    #[error("{symbol} has an invalid close {value} on {date}")]
    InvalidClose {
        /// The symbol.
        symbol: Symbol,
        /// The date of the bad close.
        date: NaiveDate,
        /// The rejected value.
        value: f64,
    },
    /// Not enough closes for the vol window.
    #[error("{symbol} has {have} closes but {need} are needed for a {window}-day window")]
    InsufficientHistory {
        /// The symbol.
        symbol: Symbol,
        /// Closes available.
        have: usize,
        /// Closes needed.
        need: usize,
        /// The configured window in returns.
        window: usize,
    },
    /// The symbols share too few dates to align a full window of returns.
    #[error("only {have} aligned returns across the symbols, {need} needed")]
    InsufficientOverlap {
        /// Aligned returns available.
        have: usize,
        /// Aligned returns needed.
        need: usize,
    },
    /// A return series has zero variance, so its correlation is undefined.
    #[error("{0} has zero return variance over the window")]
    ZeroVariance(Symbol),
    /// A computed vol was zero or not finite.
    #[error("{symbol} volatility is not usable: {value}")]
    BadVol {
        /// The symbol.
        symbol: Symbol,
        /// The value computed.
        value: f64,
    },
    /// The configuration has no dividend yield for a symbol.
    #[error("no dividend yield configured for {0}")]
    MissingDividendYield(Symbol),
    /// The market state could not be built from the seed.
    #[error(transparent)]
    Domain(#[from] DomainError),
}
