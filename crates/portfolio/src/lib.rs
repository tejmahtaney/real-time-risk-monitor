//! Positions, the position book, CSV loading, per-position valuation and Greeks, and expiry settlement.
//!
//! Loading and resolving are separate steps. [`parse_csv`] reads unresolved
//! [`PositionSpec`]s, whose strikes and expiries may be relative, and
//! [`resolve`] turns them into [`domain::Position`]s against a market state.

#![warn(missing_docs)]

mod book;
mod error;
mod resolve;
mod risk;
mod spec;
mod split;

pub use book::PositionBook;
pub use error::PortfolioError;
pub use resolve::{resolve, resolve_expiry, round_strike};
pub use risk::{PositionRisk, PositionStatus, ReportingGreeks, Settlement, SettlementEvent};
pub use spec::{
    ExpirySpec, PositionSpec, StrikeSpec, adjust_specs_for_ledger, load_csv, parse_csv,
    to_csv_bytes, to_spec, write_csv,
};
pub use split::{AppliedSplit, ledger_splits};
