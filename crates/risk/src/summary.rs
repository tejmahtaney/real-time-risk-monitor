//! The output types of risk aggregation.

use std::collections::BTreeMap;

use domain::Symbol;
use serde::{Deserialize, Serialize};

/// Risk for one underlying, summed over its active positions.
///
/// Share delta and share gamma are counts of shares of this one stock, so they
/// are meaningful here and nowhere above. Every other measure is in dollars.
/// An underlying with only expired positions still appears, with zero risk and
/// its realised P&L.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct UnderlyingRisk {
    /// The spot the positions were valued at. For an underlying with only expired positions, the settlement spot.
    pub spot: f64,
    /// Σ delta × quantity × multiplier: net shares of the underlying.
    pub share_delta: f64,
    /// Share delta × spot, summed. Adds across underlyings.
    pub dollar_delta: f64,
    /// Σ gamma × quantity × multiplier: the change in share delta per $1 move.
    pub share_gamma: f64,
    /// Σ ½ × gamma × (0.01 × spot)² × quantity × multiplier: the gamma P&L of a 1 % move. Adds across underlyings.
    pub dollar_gamma_1pct: f64,
    /// Dollars per vol point.
    pub vega: f64,
    /// Dollars per calendar day.
    pub theta: f64,
    /// Dollars per 1 % rate move.
    pub rho: f64,
    /// Σ mark × quantity × multiplier over active positions.
    pub market_value: f64,
    /// Σ (mark − entry) × quantity × multiplier over active positions.
    pub unrealised_pnl: f64,
    /// Realised P&L of settled positions.
    pub realised_pnl: f64,
    /// Σ |dollar delta| over positions. Offsetting positions do not hide each other.
    pub gross_exposure: f64,
    /// Σ |quantity × multiplier × spot|. Shares have a multiplier of 1, so this is |quantity × spot| for stock.
    pub contract_notional: f64,
    /// Number of active positions. Expired and error positions are not counted here.
    pub position_count: usize,
}

/// Portfolio totals, in dollar measures only.
///
/// There is deliberately no share delta and no share gamma field. Those are
/// counts of shares of one stock, and 100 shares of one stock plus 100 of
/// another is not 200 of anything. Because the type has nowhere to put them,
/// nobody can sum shares across underlyings by accident. Dollar delta and dollar
/// gamma are in one currency and do add.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PortfolioTotals {
    /// Net dollar delta, Σ share delta × spot over underlyings. A 1 % move in every stock changes value by about 1 % of this.
    pub dollar_delta: f64,
    /// Σ dollar gamma 1 % over underlyings.
    pub dollar_gamma_1pct: f64,
    /// Dollars per vol point.
    pub vega: f64,
    /// Dollars per calendar day.
    pub theta: f64,
    /// Dollars per 1 % rate move.
    pub rho: f64,
    /// Σ market value over underlyings.
    pub market_value: f64,
    /// Σ unrealised P&L over underlyings.
    pub unrealised_pnl: f64,
    /// Σ realised P&L over underlyings.
    pub realised_pnl: f64,
    /// Σ gross exposure over underlyings.
    pub gross_exposure: f64,
    /// Σ contract notional over underlyings.
    pub contract_notional: f64,
}

/// One underlying's share of the portfolio's gross exposure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concentration {
    /// The underlying.
    pub symbol: Symbol,
    /// Its gross exposure in dollars.
    pub gross_exposure: f64,
    /// Gross exposure ÷ portfolio gross exposure, as a fraction from 0 to 1.
    pub fraction: f64,
}

/// The result of aggregating a book.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RiskSummary {
    /// Risk per underlying, in symbol order.
    pub by_underlying: BTreeMap<Symbol, UnderlyingRisk>,
    /// Dollar-only portfolio totals.
    pub totals: PortfolioTotals,
    /// One entry per underlying, largest share first, ties in symbol order.
    /// Empty when the portfolio's gross exposure is zero.
    pub concentration: Vec<Concentration>,
    /// Positions valued normally.
    pub active_positions: usize,
    /// Positions settled at expiry.
    pub expired_positions: usize,
    /// Positions that could not be valued.
    pub error_positions: usize,
}
