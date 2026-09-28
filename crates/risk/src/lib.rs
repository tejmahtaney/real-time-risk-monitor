//! Risk aggregation and P&L attribution across the book. Limits and Value at Risk come in
//! later parts.
//!
//! [`aggregate`] is a pure function from a slice of [`portfolio::PositionRisk`]
//! rows to a [`RiskSummary`]: risk per underlying, portfolio totals in dollar
//! terms only, and each underlying's share of gross exposure.
//!
//! [`PnlAttributor`] turns a stream of per-tick [`portfolio::PositionRisk`] slices into a
//! running [`PnlReport`]: a Taylor decomposition of each tick's P&L into delta, gamma, vega,
//! theta, rho, a higher-order term and a residual, per position, per underlying and for the
//! portfolio, for today and since start.

#![warn(missing_docs)]

mod aggregate;
mod attributor;
mod pnl;
mod summary;

pub use aggregate::aggregate;
pub use attributor::PnlAttributor;
pub use pnl::{PnlBuckets, PnlLevel, PnlReport, PreviousDay};
pub use summary::{Concentration, PortfolioTotals, RiskSummary, UnderlyingRisk};
