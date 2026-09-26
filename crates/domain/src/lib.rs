//! Shared domain types: symbols, instruments, positions, market state, the clock trait and time conventions.
//!
//! Everything here is plain data with validating constructors. Rates, yields and
//! volatilities are decimals (0.05 means 5 %), timestamps are `DateTime<Utc>`
//! and time to expiry is ACT/365 calendar time. Equity options carry a
//! multiplier of 100 and expire at 16:00 New York time on the expiry date.

#![warn(missing_docs)]

mod clock;
mod error;
mod instrument;
mod market;
mod position;
mod symbol;
mod time;

pub use clock::{Clock, FixedClock};
pub use error::DomainError;
pub use instrument::{DEFAULT_MULTIPLIER, Instrument, InstrumentKind, OptionContract};
pub use market::{MarketState, UnderlyingState};
pub use position::{Position, PositionId};
pub use pricing::OptionType;
pub use symbol::Symbol;
pub use time::{MARKET_TZ, market_date, year_fraction};
