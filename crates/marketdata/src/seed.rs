//! The calibrated seed for the simulator.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{MarketState, Symbol, UnderlyingState};
use serde::{Deserialize, Serialize};

use crate::error::CalibrationError;
use crate::loader::DataSource;

/// The calibrated inputs for one underlying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolSeed {
    /// The last close in dollars.
    pub spot: f64,
    /// Realised volatility: sample standard deviation of the windowed log returns
    /// times √252, as a decimal.
    pub vol_realised: f64,
    /// EWMA volatility (`RiskMetrics`, lambda 0.94) times √252, as a decimal.
    pub vol_ewma: f64,
    /// The volatility that seeds the simulator, the one `vol_method` names.
    pub vol: f64,
    /// Continuous dividend yield as a decimal.
    pub dividend_yield: f64,
    /// Where the price history came from.
    pub source: DataSource,
}

/// Everything the simulator needs to start from real data.
///
/// `symbols` fixes the column order of [`SeedData::returns`] and the row and column
/// order of [`SeedData::correlation`]. Part 18 uses the return matrix for historical `VaR`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedData {
    /// The date of the latest close used.
    pub as_of: NaiveDate,
    /// Continuously compounded risk-free rate.
    pub rate: f64,
    /// The symbols in matrix order.
    pub symbols: Vec<Symbol>,
    /// Per-symbol calibration.
    pub underlyings: BTreeMap<Symbol, SymbolSeed>,
    /// The dates of the aligned returns, oldest first. A return is dated by its later close.
    pub return_dates: Vec<NaiveDate>,
    /// Aligned daily log returns: one row per date in `return_dates`, one column per symbol.
    pub returns: Vec<Vec<f64>>,
    /// Correlation matrix of the aligned returns. Symmetric with an exact unit diagonal.
    pub correlation: Vec<Vec<f64>>,
}

impl SeedData {
    /// The market state at `now` built from the seed: each symbol's spot, chosen vol and
    /// dividend yield, and the seed's rate.
    ///
    /// # Errors
    ///
    /// [`CalibrationError::Domain`] when a value fails the domain validation, such as a
    /// vol that is not positive.
    pub fn to_market_state(&self, now: DateTime<Utc>) -> Result<MarketState, CalibrationError> {
        let mut underlyings = Vec::with_capacity(self.underlyings.len());
        for (symbol, seed) in &self.underlyings {
            let state = UnderlyingState::new(seed.spot, seed.vol, seed.dividend_yield)?;
            underlyings.push((symbol.clone(), state));
        }
        Ok(MarketState::new(now, self.rate, underlyings)?)
    }
}
