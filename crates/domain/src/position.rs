//! Positions: a quantity of an instrument with an entry price.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, finite};
use crate::instrument::Instrument;

/// Longest accepted position id.
const MAX_ID_LEN: usize = 64;

/// A unique name for a position, such as `spy-put-hedge`.
///
/// Trimmed, 1 to 64 characters. Deserialising goes through the same check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PositionId(String);

impl PositionId {
    /// Validates and builds an id.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidPositionId`] when the trimmed input is empty or
    /// longer than 64 characters.
    ///
    /// ```
    /// use domain::PositionId;
    ///
    /// assert_eq!(PositionId::new(" spy-stock ")?.as_str(), "spy-stock");
    /// assert!(PositionId::new("").is_err());
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(input: &str) -> Result<Self, DomainError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_ID_LEN {
            return Err(DomainError::InvalidPositionId(input.to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PositionId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<PositionId> for String {
    fn from(id: PositionId) -> Self {
        id.0
    }
}

/// A holding of one instrument.
///
/// Quantity is signed: long is positive and short is negative. It counts
/// shares for equity and contracts for options. `entry_price` is per share,
/// so an option quoted at 3.20 has an entry price of 3.20 and costs
/// 3.20 × multiplier per contract. `iv_override`, when set, replaces the
/// underlying's volatility for this position only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Unique id within a book.
    pub id: PositionId,
    /// What is held.
    pub instrument: Instrument,
    /// Signed quantity: shares or contracts.
    pub quantity: f64,
    /// Entry price per unit.
    pub entry_price: f64,
    /// Volatility to use instead of the underlying's, as a decimal.
    pub iv_override: Option<f64>,
}

impl Position {
    /// Builds a position, checking that the numbers are usable.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] for a NaN or infinite quantity, entry price or
    /// override, and [`DomainError::InvalidVolOverride`] for an override that is not positive.
    ///
    /// ```
    /// use domain::{Instrument, Position, PositionId, Symbol};
    ///
    /// let p = Position::new(
    ///     PositionId::new("spy-stock")?,
    ///     Instrument::Equity { symbol: Symbol::new("SPY")? },
    ///     500.0,
    ///     600.0,
    ///     None,
    /// )?;
    /// assert_eq!(p.quantity, 500.0);
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(
        id: PositionId,
        instrument: Instrument,
        quantity: f64,
        entry_price: f64,
        iv_override: Option<f64>,
    ) -> Result<Self, DomainError> {
        finite("quantity", quantity)?;
        finite("entry_price", entry_price)?;
        if let Some(iv) = iv_override {
            finite("iv_override", iv)?;
            if iv <= 0.0 {
                return Err(DomainError::InvalidVolOverride(iv));
            }
        }
        Ok(Self {
            id,
            instrument,
            quantity,
            entry_price,
            iv_override,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::Symbol;

    fn stock() -> Instrument {
        Instrument::Equity {
            symbol: Symbol::new("SPY").unwrap(),
        }
    }

    #[test]
    fn id_validation() {
        assert!(PositionId::new("a").is_ok());
        assert!(PositionId::new(&"x".repeat(64)).is_ok());
        assert!(PositionId::new(&"x".repeat(65)).is_err());
        assert!(PositionId::new("  ").is_err());
        assert_eq!(PositionId::new("  a b ").unwrap().to_string(), "a b");
    }

    #[test]
    fn id_serde_validates() {
        assert!(serde_json::from_str::<PositionId>("\"\"").is_err());
        let id: PositionId = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(id.as_str(), "abc");
    }

    #[test]
    fn position_validation() {
        let id = || PositionId::new("p").unwrap();
        assert!(Position::new(id(), stock(), -3.0, 10.0, Some(0.3)).is_ok());
        assert!(Position::new(id(), stock(), f64::NAN, 10.0, None).is_err());
        assert!(Position::new(id(), stock(), 1.0, f64::INFINITY, None).is_err());
        assert!(Position::new(id(), stock(), 1.0, 10.0, Some(0.0)).is_err());
        assert!(Position::new(id(), stock(), 1.0, 10.0, Some(-0.2)).is_err());
        assert!(Position::new(id(), stock(), 1.0, 10.0, Some(f64::NAN)).is_err());
    }
}
