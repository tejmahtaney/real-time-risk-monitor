//! Per-position valuation output: [`PositionRisk`], reporting Greeks and settlement records.

use chrono::{DateTime, Utc};
use domain::{
    Instrument, InstrumentKind, MarketState, OptionType, Position, PositionId, Symbol,
    year_fraction,
};
use pricing::{BsInputs, Greeks};
use serde::{Deserialize, Serialize};

/// A position's Greeks in reporting units.
///
/// Each figure covers the whole position, so it includes quantity × multiplier.
/// Only the dollar measures add across underlyings. Share delta and share
/// gamma are counts of shares of one stock, which do not add to another
/// stock's, so they are meaningful per underlying only.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ReportingGreeks {
    /// Delta × quantity × multiplier: shares of the underlying the position behaves like.
    pub share_delta: f64,
    /// Share delta × spot. Adds across underlyings.
    pub dollar_delta: f64,
    /// Gamma × quantity × multiplier: the change in share delta per $1 move.
    pub share_gamma: f64,
    /// ½ × gamma × (0.01 × spot)² × quantity × multiplier: the gamma P&L of a 1 % move.
    /// Adds across underlyings.
    pub dollar_gamma_1pct: f64,
    /// Raw vega ÷ 100 × quantity × multiplier: dollars per vol point.
    pub vega: f64,
    /// Raw theta ÷ 365 × quantity × multiplier: dollars per calendar day.
    pub theta: f64,
    /// Raw rho ÷ 100 × quantity × multiplier: dollars per 1 % rate move.
    pub rho: f64,
}

impl ReportingGreeks {
    /// Scales raw per-unit Greeks to a whole position.
    ///
    /// With s = quantity × multiplier: share delta = delta × s, dollar delta =
    /// share delta × spot, share gamma = gamma × s, dollar gamma 1 % =
    /// ½ × gamma × (0.01 × spot)² × s, vega = vega ÷ 100 × s, theta =
    /// theta ÷ 365 × s and rho = rho ÷ 100 × s.
    ///
    /// ```
    /// use portfolio::ReportingGreeks;
    /// use pricing::Greeks;
    ///
    /// let raw = Greeks {
    ///     price: 5.0, delta: 0.3, gamma: 0.01, vega: 20.0,
    ///     theta: -10.0, rho: 4.0, vanna: 0.0, volga: 0.0,
    /// };
    /// // One short contract of 100 shares.
    /// let g = ReportingGreeks::scale(&raw, -1.0, 100.0, 200.0);
    /// assert!((g.share_delta + 30.0).abs() < 1e-12);
    /// assert!((g.dollar_delta + 6000.0).abs() < 1e-9);
    /// ```
    pub fn scale(raw: &Greeks, quantity: f64, multiplier: f64, spot: f64) -> Self {
        let size = quantity * multiplier;
        let share_delta = raw.delta * size;
        let one_percent_move = 0.01 * spot;
        Self {
            share_delta,
            dollar_delta: share_delta * spot,
            share_gamma: raw.gamma * size,
            dollar_gamma_1pct: 0.5 * raw.gamma * one_percent_move * one_percent_move * size,
            vega: raw.vega / 100.0 * size,
            theta: raw.theta / 365.0 * size,
            rho: raw.rho / 100.0 * size,
        }
    }
}

/// Where a position stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum PositionStatus {
    /// Valued normally.
    Active,
    /// Settled at expiry. Zero Greeks, kept for display, skipped by risk.
    Expired,
    /// Could not be valued. The reason is in plain words.
    Error {
        /// Why valuation failed.
        reason: String,
    },
}

/// The valuation of one position at one moment.
///
/// When `status` is [`PositionStatus::Error`] the derived numbers (spot, vol,
/// time to expiry, mark, market value, P&L and every Greek) are zero and
/// consumers must check the status first. Nothing here is ever NaN.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionRisk {
    /// The position's id.
    pub id: PositionId,
    /// A short description such as `AAPL 2026-10-16 C 230`.
    pub description: String,
    /// The underlying equity.
    pub underlying: Symbol,
    /// Equity, call or put.
    pub kind: InstrumentKind,
    /// Signed quantity: shares or contracts.
    pub quantity: f64,
    /// Units per quantity: 1 for shares, the contract multiplier for options.
    pub multiplier: f64,
    /// Spot used. For an expired position, the settlement spot.
    pub spot: f64,
    /// Volatility used, as a decimal. Zero for equity and expired positions.
    pub vol: f64,
    /// Time to expiry in years, ACT/365. Zero for equity and expired positions.
    pub time_to_expiry: f64,
    /// Value per unit: spot for shares, the model price for an option, the intrinsic value once settled.
    pub mark: f64,
    /// Mark × quantity × multiplier. Zero once settled, because the cash has been paid out.
    pub market_value: f64,
    /// Entry price per unit.
    pub entry_price: f64,
    /// (mark − entry) × quantity × multiplier. Zero once settled.
    pub unrealised_pnl: f64,
    /// (intrinsic − entry) × quantity × multiplier once settled, otherwise zero.
    pub realised_pnl: f64,
    /// Raw per-unit Greeks from `pricing`. Equity has delta 1 and the rest zero.
    pub greeks: Greeks,
    /// Whole-position Greeks in reporting units.
    pub reporting: ReportingGreeks,
    /// Active, expired or error.
    pub status: PositionStatus,
}

/// How an option was settled at expiry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settlement {
    /// When the settlement was recorded.
    pub settled_at: DateTime<Utc>,
    /// The spot it was settled against.
    pub spot: f64,
    /// Intrinsic value per unit: max(ω × (S − K), 0), where ω is +1 for a call and −1 for a put.
    pub intrinsic: f64,
    /// (intrinsic − entry) × quantity × multiplier.
    pub realised_pnl: f64,
}

/// A record that an option was settled, returned by `PositionBook::settle_expired`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettlementEvent {
    /// The settled position.
    pub position_id: PositionId,
    /// A short description such as `SPY 2026-10-02 C 600`.
    pub description: String,
    /// The underlying equity.
    pub underlying: Symbol,
    /// Call or put.
    pub kind: InstrumentKind,
    /// The strike.
    pub strike: f64,
    /// Signed number of contracts.
    pub quantity: f64,
    /// Shares per contract.
    pub multiplier: f64,
    /// Entry price per unit.
    pub entry_price: f64,
    /// When the option expired.
    pub expiry: DateTime<Utc>,
    /// When the settlement was recorded.
    pub settled_at: DateTime<Utc>,
    /// The spot it was settled against.
    pub spot: f64,
    /// Intrinsic value per unit.
    pub intrinsic: f64,
    /// (intrinsic − entry) × quantity × multiplier.
    pub realised_pnl: f64,
}

/// Values one position, turning any failure into an `Error` row.
pub(crate) fn value_position(
    position: &Position,
    settlement: Option<&Settlement>,
    market: &MarketState,
    now: DateTime<Utc>,
) -> PositionRisk {
    let mut row = base_row(position);
    match fill_row(&mut row, position, settlement, market, now) {
        Ok(()) => row,
        Err(reason) => {
            let mut error_row = base_row(position);
            error_row.status = PositionStatus::Error { reason };
            error_row
        }
    }
}

/// A row with the position's own facts and zeros for everything derived.
fn base_row(position: &Position) -> PositionRisk {
    PositionRisk {
        id: position.id.clone(),
        description: position.instrument.description(),
        underlying: position.instrument.underlying().clone(),
        kind: position.instrument.kind(),
        quantity: position.quantity,
        multiplier: position.instrument.multiplier(),
        spot: 0.0,
        vol: 0.0,
        time_to_expiry: 0.0,
        mark: 0.0,
        market_value: 0.0,
        entry_price: position.entry_price,
        unrealised_pnl: 0.0,
        realised_pnl: 0.0,
        greeks: zero_greeks(0.0),
        reporting: ReportingGreeks::default(),
        status: PositionStatus::Active,
    }
}

/// Raw Greeks with every sensitivity zero and the given price.
fn zero_greeks(price: f64) -> Greeks {
    Greeks {
        price,
        delta: 0.0,
        gamma: 0.0,
        vega: 0.0,
        theta: 0.0,
        rho: 0.0,
        vanna: 0.0,
        volga: 0.0,
    }
}

fn fill_row(
    row: &mut PositionRisk,
    position: &Position,
    settlement: Option<&Settlement>,
    market: &MarketState,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if let Some(s) = settlement {
        row.spot = s.spot;
        row.mark = s.intrinsic;
        row.realised_pnl = s.realised_pnl;
        row.greeks = zero_greeks(s.intrinsic);
        row.status = PositionStatus::Expired;
        return Ok(());
    }

    let symbol = position.instrument.underlying();
    let state = market
        .underlying(symbol)
        .ok_or_else(|| format!("no market data for {symbol}"))?;
    let size = position.quantity * row.multiplier;
    if !size.is_finite() || row.multiplier <= 0.0 {
        return Err(format!(
            "invalid quantity {} or multiplier {}",
            position.quantity, row.multiplier
        ));
    }

    let (raw, vol, time_to_expiry) = match &position.instrument {
        Instrument::Equity { .. } => {
            let mut raw = zero_greeks(state.spot());
            raw.delta = 1.0;
            (raw, 0.0, 0.0)
        }
        Instrument::Option(contract) => {
            let expiry = contract.expiry_datetime();
            if now >= expiry {
                return Err(format!("expired at {expiry} and not settled"));
            }
            let vol = position.iv_override.unwrap_or_else(|| state.vol());
            let time_to_expiry = year_fraction(now, expiry);
            let inputs = BsInputs::new(
                state.spot(),
                contract.strike,
                time_to_expiry,
                market.rate(),
                state.dividend_yield(),
                vol,
            )
            .map_err(|e| e.to_string())?;
            (
                pricing::greeks(contract.option_type, &inputs),
                vol,
                time_to_expiry,
            )
        }
    };

    let reporting = ReportingGreeks::scale(&raw, position.quantity, row.multiplier, state.spot());
    let market_value = raw.price * size;
    let unrealised_pnl = (raw.price - position.entry_price) * size;
    if !(market_value.is_finite() && unrealised_pnl.is_finite()) {
        return Err("valuation is not finite".to_owned());
    }

    row.spot = state.spot();
    row.vol = vol;
    row.time_to_expiry = time_to_expiry;
    row.mark = raw.price;
    row.market_value = market_value;
    row.unrealised_pnl = unrealised_pnl;
    row.greeks = raw;
    row.reporting = reporting;
    Ok(())
}

/// Intrinsic value per unit at `spot`: max(ω × (S − K), 0).
pub(crate) fn intrinsic(option_type: OptionType, strike: f64, spot: f64) -> f64 {
    match option_type {
        OptionType::Call => (spot - strike).max(0.0),
        OptionType::Put => (strike - spot).max(0.0),
    }
}
