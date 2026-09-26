//! The price feed abstraction: what a feed emits and how it can fail.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::{ClockStep, DomainError, MarketState, StepKind, Symbol};
use thiserror::Error;

use crate::config::ConfigError;

/// A spot price and its volatility at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quote {
    /// Spot price in dollars.
    pub spot: f64,
    /// Annualised volatility as a decimal.
    pub vol: f64,
}

/// The state of every symbol at the end of one clock step.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketTick {
    /// The simulated time, which is the end of the step.
    pub time: DateTime<Utc>,
    /// The kind of step that produced the tick.
    pub kind: StepKind,
    /// Spot and vol per symbol.
    pub quotes: BTreeMap<Symbol, Quote>,
}

impl MarketTick {
    /// Writes the tick into `market`: sets its time and each symbol's spot and vol, and
    /// leaves the rate and dividend yields alone.
    ///
    /// # Errors
    ///
    /// [`DomainError::UnknownSymbol`] when the tick names a symbol the market does not
    /// hold, or the [`UnderlyingState`](domain::UnderlyingState) errors for a bad value.
    /// The market is unchanged on error.
    pub fn apply_to(&self, market: &mut MarketState) -> Result<(), DomainError> {
        market.update(
            self.time,
            self.quotes
                .iter()
                .map(|(symbol, quote)| (symbol.clone(), quote.spot, quote.vol)),
        )
    }
}

/// A source of prices that moves one clock step at a time.
///
/// Feeds are synchronous and step-based. The engine calls [`step`](Self::step) on a timer
/// and the backtester calls it in a loop, with the same code either way.
pub trait PriceFeed {
    /// Moves every symbol on by `step` and returns the new state.
    ///
    /// # Errors
    ///
    /// [`FeedError`] when the step is invalid or the move produces a value that is not a
    /// positive finite number. The feed's prices are unchanged on error.
    fn step(&mut self, step: &ClockStep) -> Result<MarketTick, FeedError>;
}

/// A price feed could not be built or stepped.
#[derive(Debug, Error)]
pub enum FeedError {
    /// The seed data holds no symbols.
    #[error("seed data has no symbols")]
    EmptySeed,
    /// A symbol's seed is missing or not usable.
    #[error("bad seed for {symbol}: {message}")]
    InvalidSeed {
        /// The symbol.
        symbol: Symbol,
        /// What is wrong.
        message: String,
    },
    /// The correlation matrix is malformed: wrong shape, not symmetric, no unit diagonal,
    /// an entry outside [−1, 1], or a non-finite entry.
    #[error("invalid correlation matrix: {0}")]
    InvalidCorrelation(String),
    /// The clock step cannot drive a feed: non-finite or negative trading time, or
    /// trading time on an overnight step.
    #[error("invalid clock step: {0}")]
    InvalidStep(String),
    /// A move produced a spot or vol that is not a positive finite number.
    #[error("{quantity} of {symbol} became {value}")]
    NonFinite {
        /// The symbol.
        symbol: Symbol,
        /// Which quantity, `spot` or `vol`.
        quantity: &'static str,
        /// The offending value.
        value: f64,
    },
    /// The simulation configuration is invalid.
    #[error(transparent)]
    Config(#[from] ConfigError),
}
