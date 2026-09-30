//! P&L attribution output types: [`PnlBuckets`], the per-level [`PnlLevel`] and the
//! API-facing [`PnlReport`] and [`PreviousDay`].

use std::collections::BTreeMap;
use std::ops::{Add, AddAssign};

use chrono::NaiveDate;
use domain::{PositionId, Symbol};
use serde::{Deserialize, Serialize};

/// One tick's P&L for one position, underlying or the portfolio, split into a Taylor
/// decomposition of the Greeks plus an honest residual.
///
/// For a position with Greeks (Δ, Γ, ν, Θ, ρ, vanna, volga) at the start of the tick,
/// spot move dS, vol move dσ, calendar time dt, rate move dr and
/// scale = quantity × multiplier:
///
/// - `delta` = Δ·dS·scale
/// - `gamma` = ½·Γ·dS²·scale
/// - `vega` = ν·dσ·scale
/// - `theta` = Θ·dt·scale
/// - `rho` = ρ·dr·scale
/// - `higher_order` = (vanna·dS·dσ + ½·volga·dσ²)·scale
/// - `dividends` = the cash a stock position received on an ex-date, quantity × amount
/// - `actual` = (V₁ − V₀)·scale, plus that cash, from full revaluation
/// - `residual` = `actual` − [`explained`](Self::explained)
///
/// **Why `dividends` is its own bucket.** On the ex-date the stock price drops by the
/// dividend. That drop is a real fall in the position's value and shows up in `delta` (and
/// in `actual`), and the holder is paid the same amount in cash. Without a bucket for the
/// cash, a stock holder's P&L would show a loss of quantity × dividend that never
/// happened economically. The cash goes into `dividends` and into `actual` together, so
/// the total is continuous across the ex-date and `residual` is untouched.
///
/// The residual is not an error to hide: it holds the third-order and cross terms the
/// expansion leaves out, and it grows with step size and near expiry. Every field here
/// is already scaled to the whole position (or the sum of several), so buckets add
/// directly across positions, underlyings and ticks.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct PnlBuckets {
    /// Δ·dS·scale.
    pub delta: f64,
    /// ½·Γ·dS²·scale.
    pub gamma: f64,
    /// ν·dσ·scale.
    pub vega: f64,
    /// Θ·dt·scale.
    pub theta: f64,
    /// ρ·dr·scale. Zero while the risk-free rate is constant across a session, as it is today.
    pub rho: f64,
    /// (vanna·dS·dσ + ½·volga·dσ²)·scale: the cross and pure-vol-convexity terms.
    pub higher_order: f64,
    /// Cash dividends received on an ex-date: quantity × amount for a stock position, so a
    /// short position shows a negative figure. It is realised P&L, paid in cash.
    pub dividends: f64,
    /// `actual` − `explained()`: what the buckets above do not account for.
    pub residual: f64,
    /// (V₁ − V₀)·scale from full revaluation, plus any dividend cash. The true change in
    /// value for the tick.
    pub actual: f64,
}

impl PnlBuckets {
    /// The sum of the modelled buckets:
    /// `delta + gamma + vega + theta + rho + higher_order + dividends`.
    ///
    /// `residual` is always `actual - explained()`, so this and `residual` together
    /// account for the whole of `actual`.
    pub fn explained(&self) -> f64 {
        self.delta
            + self.gamma
            + self.vega
            + self.theta
            + self.rho
            + self.higher_order
            + self.dividends
    }
}

impl Add for PnlBuckets {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            delta: self.delta + rhs.delta,
            gamma: self.gamma + rhs.gamma,
            vega: self.vega + rhs.vega,
            theta: self.theta + rhs.theta,
            rho: self.rho + rhs.rho,
            higher_order: self.higher_order + rhs.higher_order,
            dividends: self.dividends + rhs.dividends,
            residual: self.residual + rhs.residual,
            actual: self.actual + rhs.actual,
        }
    }
}

impl AddAssign for PnlBuckets {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

/// One level's running P&L: today's session and everything since the attributor started.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct PnlLevel {
    /// Buckets accumulated since the current session opened.
    pub today: PnlBuckets,
    /// Buckets accumulated since the attributor was created.
    pub since_start: PnlBuckets,
}

/// The previous session's final P&L, kept once a new session opens.
///
/// Only that day's totals are kept, not its own `since_start` or a further `previous_day`,
/// so this type does not nest indefinitely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PreviousDay {
    /// The New York date of the session these totals belong to.
    pub session_date: NaiveDate,
    /// The whole portfolio's P&L for that session.
    pub portfolio: PnlBuckets,
    /// Each underlying's P&L for that session.
    pub by_underlying: BTreeMap<Symbol, PnlBuckets>,
    /// Each position's P&L for that session.
    pub by_position: BTreeMap<PositionId, PnlBuckets>,
}

/// The API-facing P&L attribution report: today and since-start buckets at every level,
/// the previous session's totals, and the current session date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct PnlReport {
    /// The New York date of the current session.
    pub session_date: NaiveDate,
    /// The whole portfolio's P&L.
    pub portfolio: PnlLevel,
    /// Each underlying's P&L.
    pub by_underlying: BTreeMap<Symbol, PnlLevel>,
    /// Each position's P&L. A position no longer in the book keeps what it contributed.
    pub by_position: BTreeMap<PositionId, PnlLevel>,
    /// The previous session's totals, once a session roll has happened.
    pub previous_day: Option<PreviousDay>,
}
