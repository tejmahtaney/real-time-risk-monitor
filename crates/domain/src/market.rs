//! Market state: the risk-free rate and each underlying's spot, volatility and dividend yield.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{DomainError, finite};
use crate::symbol::Symbol;

/// The market data for one underlying at one moment.
///
/// Fields are private and the constructor validates them, so a value that
/// exists is usable: spot and volatility are positive and every number is
/// finite. Deserialising goes through the same check. The volatility and
/// dividend yield are decimals (0.25 means 25 %).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawUnderlying")]
pub struct UnderlyingState {
    spot: f64,
    vol: f64,
    dividend_yield: f64,
}

#[derive(Deserialize)]
struct RawUnderlying {
    spot: f64,
    vol: f64,
    dividend_yield: f64,
}

impl TryFrom<RawUnderlying> for UnderlyingState {
    type Error = DomainError;

    fn try_from(raw: RawUnderlying) -> Result<Self, Self::Error> {
        Self::new(raw.spot, raw.vol, raw.dividend_yield)
    }
}

impl UnderlyingState {
    /// Validates and builds the state.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] for a NaN or infinite input,
    /// [`DomainError::InvalidSpot`] or [`DomainError::InvalidVol`] when spot or
    /// volatility is not positive.
    ///
    /// ```
    /// use domain::UnderlyingState;
    ///
    /// assert!(UnderlyingState::new(230.0, 0.25, 0.005).is_ok());
    /// assert!(UnderlyingState::new(0.0, 0.25, 0.0).is_err());
    /// ```
    pub fn new(spot: f64, vol: f64, dividend_yield: f64) -> Result<Self, DomainError> {
        finite("spot", spot)?;
        finite("vol", vol)?;
        finite("dividend_yield", dividend_yield)?;
        if spot <= 0.0 {
            return Err(DomainError::InvalidSpot(spot));
        }
        if vol <= 0.0 {
            return Err(DomainError::InvalidVol(vol));
        }
        Ok(Self {
            spot,
            vol,
            dividend_yield,
        })
    }

    /// Spot price in dollars.
    pub fn spot(&self) -> f64 {
        self.spot
    }

    /// Annualised volatility as a decimal.
    pub fn vol(&self) -> f64 {
        self.vol
    }

    /// Continuous dividend yield as a decimal.
    pub fn dividend_yield(&self) -> f64 {
        self.dividend_yield
    }
}

/// A snapshot of the market: a timestamp, one continuously compounded rate and
/// the state of each underlying.
///
/// Underlyings are kept in a `BTreeMap`, so iteration order is by symbol and
/// therefore deterministic. Deserialising validates the rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawMarket")]
pub struct MarketState {
    as_of: DateTime<Utc>,
    rate: f64,
    underlyings: BTreeMap<Symbol, UnderlyingState>,
}

#[derive(Deserialize)]
struct RawMarket {
    as_of: DateTime<Utc>,
    rate: f64,
    underlyings: BTreeMap<Symbol, UnderlyingState>,
}

impl TryFrom<RawMarket> for MarketState {
    type Error = DomainError;

    fn try_from(raw: RawMarket) -> Result<Self, Self::Error> {
        Self::new(raw.as_of, raw.rate, raw.underlyings)
    }
}

impl MarketState {
    /// Builds a market state. A later entry for the same symbol replaces an earlier one.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] when the rate is NaN or infinite. Each
    /// [`UnderlyingState`] was validated when it was built.
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use domain::{MarketState, Symbol, UnderlyingState};
    ///
    /// let t = Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
    /// let market = MarketState::new(
    ///     t,
    ///     0.04,
    ///     [(Symbol::new("SPY")?, UnderlyingState::new(600.0, 0.16, 0.013)?)],
    /// )?;
    /// assert_eq!(market.underlying(&Symbol::new("spy")?).map(|u| u.spot()), Some(600.0));
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(
        as_of: DateTime<Utc>,
        rate: f64,
        underlyings: impl IntoIterator<Item = (Symbol, UnderlyingState)>,
    ) -> Result<Self, DomainError> {
        finite("rate", rate)?;
        Ok(Self {
            as_of,
            rate,
            underlyings: underlyings.into_iter().collect(),
        })
    }

    /// When the market state was observed.
    pub fn as_of(&self) -> DateTime<Utc> {
        self.as_of
    }

    /// The continuously compounded risk-free rate as a decimal.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The state of one underlying, or `None` when the market has no data for it.
    pub fn underlying(&self, symbol: &Symbol) -> Option<&UnderlyingState> {
        self.underlyings.get(symbol)
    }

    /// All underlyings in symbol order.
    pub fn iter(&self) -> impl Iterator<Item = (&Symbol, &UnderlyingState)> {
        self.underlyings.iter()
    }

    /// The symbols the market has data for, in order.
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.underlyings.keys()
    }

    /// Adds or replaces one underlying.
    pub fn insert(&mut self, symbol: Symbol, state: UnderlyingState) {
        self.underlyings.insert(symbol, state);
    }

    /// Moves the market to `as_of` with new spots and vols, keeping each dividend yield
    /// and the rate. All or nothing: when any update fails, the market is unchanged.
    ///
    /// # Errors
    ///
    /// [`DomainError::UnknownSymbol`] for a symbol the market does not hold, and the
    /// [`UnderlyingState::new`] errors for a bad spot or vol.
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use domain::{MarketState, Symbol, UnderlyingState};
    ///
    /// let t = Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
    /// let spy = Symbol::new("SPY")?;
    /// let mut market = MarketState::new(t, 0.04, [(spy.clone(), UnderlyingState::new(600.0, 0.16, 0.013)?)])?;
    /// let later = Utc.with_ymd_and_hms(2026, 10, 1, 14, 31, 0).unwrap();
    /// market.update(later, [(spy.clone(), 601.5, 0.17)])?;
    /// let state = market.underlying(&spy).unwrap();
    /// assert_eq!((state.spot(), state.vol(), state.dividend_yield()), (601.5, 0.17, 0.013));
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn update(
        &mut self,
        as_of: DateTime<Utc>,
        updates: impl IntoIterator<Item = (Symbol, f64, f64)>,
    ) -> Result<(), DomainError> {
        let mut staged = Vec::new();
        for (symbol, spot, vol) in updates {
            let current = self
                .underlyings
                .get(&symbol)
                .ok_or_else(|| DomainError::UnknownSymbol(symbol.to_string()))?;
            let state = UnderlyingState::new(spot, vol, current.dividend_yield())?;
            staged.push((symbol, state));
        }
        self.as_of = as_of;
        self.underlyings.extend(staged);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn t() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap()
    }

    #[test]
    fn update_replaces_spot_and_vol_and_keeps_the_yield() {
        let later = t() + chrono::Duration::minutes(1);
        let mut market = MarketState::new(
            t(),
            0.04,
            [
                (
                    sym("SPY"),
                    UnderlyingState::new(600.0, 0.16, 0.013).unwrap(),
                ),
                (
                    sym("AAPL"),
                    UnderlyingState::new(230.0, 0.25, 0.005).unwrap(),
                ),
            ],
        )
        .unwrap();
        market.update(later, [(sym("SPY"), 601.0, 0.17)]).unwrap();
        assert_eq!(market.as_of(), later);
        let spy = market.underlying(&sym("SPY")).unwrap();
        assert_relative_eq!(spy.spot(), 601.0);
        assert_relative_eq!(spy.vol(), 0.17);
        assert_relative_eq!(spy.dividend_yield(), 0.013);
        assert_relative_eq!(market.underlying(&sym("AAPL")).unwrap().spot(), 230.0);
    }

    #[test]
    fn a_failed_update_changes_nothing() {
        let later = t() + chrono::Duration::minutes(1);
        let mut market = MarketState::new(
            t(),
            0.04,
            [
                (
                    sym("SPY"),
                    UnderlyingState::new(600.0, 0.16, 0.013).unwrap(),
                ),
                (
                    sym("AAPL"),
                    UnderlyingState::new(230.0, 0.25, 0.005).unwrap(),
                ),
            ],
        )
        .unwrap();
        let before = market.clone();
        let bad_vol = market.update(
            later,
            [(sym("SPY"), 601.0, 0.17), (sym("AAPL"), 231.0, 0.0)],
        );
        assert_eq!(bad_vol, Err(DomainError::InvalidVol(0.0)));
        assert_eq!(market, before);
        let unknown = market.update(later, [(sym("MSFT"), 500.0, 0.3)]);
        assert_eq!(unknown, Err(DomainError::UnknownSymbol("MSFT".into())));
        assert_eq!(market, before);
    }

    #[test]
    fn underlying_validation() {
        assert!(UnderlyingState::new(100.0, 0.2, 0.0).is_ok());
        assert!(UnderlyingState::new(100.0, 0.2, -0.01).is_ok());
        assert_eq!(
            UnderlyingState::new(0.0, 0.2, 0.0),
            Err(DomainError::InvalidSpot(0.0))
        );
        assert_eq!(
            UnderlyingState::new(-1.0, 0.2, 0.0),
            Err(DomainError::InvalidSpot(-1.0))
        );
        assert_eq!(
            UnderlyingState::new(100.0, 0.0, 0.0),
            Err(DomainError::InvalidVol(0.0))
        );
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(UnderlyingState::new(bad, 0.2, 0.0).is_err());
            assert!(UnderlyingState::new(100.0, bad, 0.0).is_err());
            assert!(UnderlyingState::new(100.0, 0.2, bad).is_err());
        }
    }

    #[test]
    fn getters_return_the_inputs() {
        let u = UnderlyingState::new(230.0, 0.25, 0.005).unwrap();
        assert_relative_eq!(u.spot(), 230.0);
        assert_relative_eq!(u.vol(), 0.25);
        assert_relative_eq!(u.dividend_yield(), 0.005);
    }

    #[test]
    fn market_rejects_non_finite_rate() {
        assert!(MarketState::new(t(), f64::NAN, []).is_err());
        assert!(MarketState::new(t(), f64::INFINITY, []).is_err());
        assert!(MarketState::new(t(), -0.005, []).is_ok());
    }

    #[test]
    fn market_lookup_and_order() {
        let u = |s| UnderlyingState::new(s, 0.2, 0.0).unwrap();
        let mut m = MarketState::new(
            t(),
            0.04,
            [(sym("MSFT"), u(400.0)), (sym("AAPL"), u(230.0))],
        )
        .unwrap();
        assert_eq!(
            m.symbols().map(Symbol::as_str).collect::<Vec<_>>(),
            ["AAPL", "MSFT"]
        );
        assert!(m.underlying(&sym("NVDA")).is_none());
        m.insert(sym("NVDA"), u(130.0));
        assert_eq!(m.iter().count(), 3);
        assert_relative_eq!(m.underlying(&sym("NVDA")).unwrap().spot(), 130.0);
        assert_eq!(m.as_of(), t());
        assert_relative_eq!(m.rate(), 0.04);
    }

    #[test]
    fn serde_round_trip() {
        let m = MarketState::new(
            t(),
            0.04,
            [(
                sym("SPY"),
                UnderlyingState::new(600.0, 0.16, 0.013).unwrap(),
            )],
        )
        .unwrap();
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<MarketState>(&json).unwrap(), m);
    }

    #[test]
    fn serde_rejects_bad_data() {
        let bad_spot = r#"{"spot":-1.0,"vol":0.2,"dividend_yield":0.0}"#;
        assert!(serde_json::from_str::<UnderlyingState>(bad_spot).is_err());
        let bad_vol = r#"{"spot":100.0,"vol":0.0,"dividend_yield":0.0}"#;
        assert!(serde_json::from_str::<UnderlyingState>(bad_vol).is_err());
        let bad_symbol = r#"{"as_of":"2026-10-01T14:30:00Z","rate":0.04,"underlyings":{"not valid":{"spot":1.0,"vol":0.2,"dividend_yield":0.0}}}"#;
        assert!(serde_json::from_str::<MarketState>(bad_symbol).is_err());
    }
}
