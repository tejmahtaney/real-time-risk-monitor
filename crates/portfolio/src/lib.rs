//! Positions, the position book, CSV loading, per-position valuation and Greeks, and expiry settlement.
//!
//! Loading and resolving are separate steps. [`parse_csv`] reads unresolved
//! [`PositionSpec`]s, whose strikes and expiries may be relative, and
//! [`resolve`] turns them into [`domain::Position`]s against a market state.

#![warn(missing_docs)]

mod error;
mod resolve;
mod spec;

pub use error::PortfolioError;
pub use resolve::{resolve, resolve_expiry, round_strike};
pub use spec::{ExpirySpec, PositionSpec, StrikeSpec, load_csv, parse_csv};
