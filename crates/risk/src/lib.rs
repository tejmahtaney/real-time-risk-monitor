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
//!
//! [`LimitMonitor`] evaluates a [`RiskSummary`] against a [`LimitsConfig`] on every tick,
//! tracking each limit's utilisation, warning and breach state with hysteresis and raising
//! an [`AlertEvent`] only when a state changes.
//!
//! [`VarCalculator`] computes one-day Value at Risk and Expected Shortfall by three
//! methods (delta-normal, delta-gamma Monte Carlo, historical simulation) into a
//! [`VarReport`], per Part 18.

#![warn(missing_docs)]

mod aggregate;
mod attributor;
mod limits;
mod monitor;
mod pnl;
mod risk_config;
mod summary;
mod var;

pub use aggregate::aggregate;
pub use attributor::PnlAttributor;
pub use limits::{LimitsConfig, LimitsConfigError, PortfolioLimits, UnderlyingDeltaLimit};
pub use monitor::{AlertEvent, LimitKind, LimitMonitor, LimitState, LimitStatus, Scope};
pub use pnl::{PnlBuckets, PnlLevel, PnlReport, PreviousDay};
pub use risk_config::{RiskConfig, RiskConfigError};
pub use summary::{Concentration, PortfolioTotals, RiskSummary, UnderlyingRisk};
pub use var::{VarCalculator, VarMethod, VarMethodReport, VarReport};
