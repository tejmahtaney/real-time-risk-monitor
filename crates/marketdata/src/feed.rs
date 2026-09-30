//! The price feed abstraction: what a feed emits and how it can fail.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::{ClockStep, DomainError, MarketState, SpotAdjustment, StepKind, Symbol};
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

    /// Applies a split or a cash dividend to one symbol's spot, as at a session open.
    ///
    /// The feed keeps its own price state and emits absolute spots every step, so a
    /// corporate action applied only to the market state would be overwritten by the next
    /// tick. The engine therefore applies each action to the feed and to the market with
    /// the same [`SpotAdjustment`]. Returns are unaffected: a split is a change of unit.
    ///
    /// # Errors
    ///
    /// [`FeedError::Unsupported`] for a feed that cannot do this (the default),
    /// [`FeedError::UnknownSymbol`] for a symbol the feed does not hold, and
    /// [`FeedError::Domain`] when the adjustment would leave the spot at or below zero.
    /// The feed's prices are unchanged on error.
    fn adjust_spot(
        &mut self,
        symbol: &Symbol,
        adjustment: SpotAdjustment,
    ) -> Result<(), FeedError> {
        let _ = (symbol, adjustment);
        Err(FeedError::Unsupported("spot adjustments"))
    }
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
    /// The feed does not support the named operation.
    #[error("this feed does not support {0}")]
    Unsupported(&'static str),
    /// A spot adjustment named a symbol the feed does not hold.
    #[error("the feed has no symbol {0}")]
    UnknownSymbol(Symbol),
    /// A spot adjustment was invalid.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// The simulation configuration is invalid.
    #[error(transparent)]
    Config(#[from] ConfigError),
}
