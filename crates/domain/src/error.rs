//! Validation errors for the domain types.

use thiserror::Error;

/// A domain value broke one of its validation rules.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainError {
    /// A symbol must be 1 to 10 characters of A to Z, digits, `.` or `-`.
    #[error("invalid symbol {0:?}: expected 1 to 10 characters of A-Z, 0-9, '.' or '-'")]
    InvalidSymbol(String),
    /// A position id must be 1 to 64 characters once trimmed.
    #[error("invalid position id {0:?}: expected 1 to 64 characters")]
    InvalidPositionId(String),
    /// The named number is NaN or infinite.
    #[error("{field} must be finite, got {value}")]
    NonFinite {
        /// Name of the field.
        field: &'static str,
        /// The rejected value.
        value: f64,
    },
    /// Spot must be strictly positive.
    #[error("spot must be positive, got {0}")]
    InvalidSpot(f64),
    /// Volatility must be strictly positive.
    #[error("volatility must be positive, got {0}")]
    InvalidVol(f64),
    /// Strike must be strictly positive.
    #[error("strike must be positive, got {0}")]
    InvalidStrike(f64),
    /// The contract multiplier must be strictly positive.
    #[error("multiplier must be positive, got {0}")]
    InvalidMultiplier(f64),
    /// An implied-volatility override must be strictly positive.
    #[error("volatility override must be positive, got {0}")]
    InvalidVolOverride(f64),
    /// A clock speed must be finite and strictly positive.
    #[error("clock speed must be finite and positive, got {0}")]
    InvalidSpeed(f64),
    /// A market update named a symbol the market has no data for.
    #[error("market has no data for {0}")]
    UnknownSymbol(String),
    /// A split ratio must be finite, positive and not 1.
    #[error("split ratio must be finite, positive and not 1, got {0}")]
    InvalidSplitRatio(f64),
    /// A cash dividend must be finite and strictly positive.
    #[error("dividend amount must be positive, got {0}")]
    InvalidDividendAmount(f64),
    /// A spot adjustment would leave the spot at or below zero.
    #[error("adjusting spot {spot} would leave a non-positive price {result}")]
    NonPositiveAdjustedSpot {
        /// Spot before the adjustment.
        spot: f64,
        /// The rejected result.
        result: f64,
    },
    /// The escrowed spot S − ΣPV(D) is not positive, so the option cannot be priced.
    #[error("escrowed spot is not positive: spot {spot} minus dividend present value {pv}")]
    EscrowedSpotNonPositive {
        /// The spot the escrow was taken from.
        spot: f64,
        /// The present value of the dividends before expiry.
        pv: f64,
    },
}

/// Checks that `value` is finite, naming `field` on failure.
pub(crate) fn finite(field: &'static str, value: f64) -> Result<(), DomainError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DomainError::NonFinite { field, value })
    }
}
