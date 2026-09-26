//! Risk aggregation across the book. P&L attribution, limits and Value at Risk come in later parts.
//!
//! [`aggregate`] is a pure function from a slice of [`portfolio::PositionRisk`]
//! rows to a [`RiskSummary`]: risk per underlying, portfolio totals in dollar
//! terms only, and each underlying's share of gross exposure.

#![warn(missing_docs)]

mod aggregate;
mod summary;

pub use aggregate::aggregate;
pub use summary::{Concentration, PortfolioTotals, RiskSummary, UnderlyingRisk};
