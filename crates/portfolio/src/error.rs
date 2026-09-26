//! Errors from loading, resolving and managing a portfolio.

use domain::{DomainError, PositionId, Symbol};
use pricing::PricingError;
use thiserror::Error;

/// Something went wrong loading, resolving or managing positions.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PortfolioError {
    /// A CSV row or header could not be parsed. `line` counts physical lines from 1.
    #[error("line {line}, column {column}: {message}")]
    Csv {
        /// The line number in the file, starting at 1.
        line: u64,
        /// The column name, or `(row)` when the whole row is at fault.
        column: String,
        /// What is wrong.
        message: String,
    },
    /// The portfolio file could not be read.
    #[error("cannot read {path}: {message}")]
    Io {
        /// The path that failed.
        path: String,
        /// The operating system error.
        message: String,
    },
    /// A position with this id is already in the book.
    #[error("duplicate position id {0}")]
    DuplicateId(PositionId),
    /// No position with this id is in the book.
    #[error("unknown position id {0}")]
    UnknownPosition(PositionId),
    /// The market state has no data for the position's underlying.
    #[error("position {id}: no market data for {symbol}")]
    MissingUnderlying {
        /// The position.
        id: PositionId,
        /// The underlying that is missing.
        symbol: Symbol,
    },
    /// An option position is missing its strike or expiry.
    #[error("position {id}: an option needs a {field}")]
    MissingField {
        /// The position.
        id: PositionId,
        /// `strike` or `expiry`.
        field: &'static str,
    },
    /// The strike resolved to zero or less, or to a non-finite number.
    #[error("position {id}: strike resolved to {strike}, which is not positive")]
    InvalidStrike {
        /// The position.
        id: PositionId,
        /// The rounded strike.
        strike: f64,
    },
    /// A relative expiry ran past the range of dates.
    #[error("position {id}: expiry is out of range")]
    InvalidExpiry {
        /// The position.
        id: PositionId,
    },
    /// A domain value was invalid.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// The pricing inputs were invalid.
    #[error(transparent)]
    Pricing(#[from] PricingError),
}
