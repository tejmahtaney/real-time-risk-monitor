//! The immutable, publishable state of the engine at one tick.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use domain::Symbol;
use portfolio::PositionRisk;
use risk::{AlertEvent, LimitStatus, PnlReport, RiskSummary};
use serde::{Deserialize, Serialize};

/// Whether the feed is producing usable data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedStatus {
    /// The last tick's feed step and market update both succeeded.
    Ok,
    /// The last tick's feed step or market update failed. The market's prices are
    /// exactly what they were on the last tick that succeeded, but every other field on
    /// this snapshot (positions, risk, P&L, limits) was still computed fresh against
    /// those stale prices and the real time that has passed: settlement, theta decay and
    /// the overnight day roll never stop just because the feed has.
    Degraded,
}

/// One underlying's market data for a snapshot: spot, vol and the change since the
/// session's open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolSnapshot {
    /// The symbol.
    pub symbol: Symbol,
    /// Current spot.
    pub spot: f64,
    /// Current volatility, as a decimal.
    pub vol: f64,
    /// Fraction change in spot since this session's open: `(spot - open) / open`.
    pub change_since_open: f64,
}

/// An immutable snapshot of the whole engine's state at one tick, published on the
/// `watch` channel. Building one is the last step of [`crate::Engine::tick`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskSnapshot {
    /// Monotonically increasing tick counter, starting at 0. Increments on every tick,
    /// including a degraded one.
    pub seq: u64,
    /// Real-world time this snapshot was built.
    pub wall_time: DateTime<Utc>,
    /// Simulated time this snapshot represents.
    pub sim_time: DateTime<Utc>,
    /// The trading session this tick belongs to.
    pub session_date: NaiveDate,
    /// Market data per underlying, in symbol order.
    pub market: BTreeMap<Symbol, SymbolSnapshot>,
    /// Every position's valuation.
    pub positions: Vec<PositionRisk>,
    /// Aggregated risk.
    pub summary: RiskSummary,
    /// Running P&L attribution.
    pub pnl: PnlReport,
    /// The limits currently in `Warning` or `Breach`. `risk::LimitMonitor` only tracks
    /// state for limits that have left `Ok` at least once, so this is also the full set
    /// of statuses worth surfacing — a limit that has never left `Ok` carries no
    /// information beyond "fine".
    pub active_alerts: Vec<LimitStatus>,
    /// Up to the last 200 alert state-change events.
    pub alert_history: Vec<AlertEvent>,
    /// The clock's current speed multiplier.
    pub clock_speed: f64,
    /// Whether the clock is paused.
    pub paused: bool,
    /// Whether the feed produced usable data on the last tick.
    pub feed_status: FeedStatus,
    /// Microseconds spent computing this tick, measured with `Instant` around the whole
    /// eight-step cycle.
    pub compute_micros: u64,
}
