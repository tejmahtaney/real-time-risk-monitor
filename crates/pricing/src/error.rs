//! Validation errors for pricing inputs.

use thiserror::Error;

/// A pricing input broke one of the validation rules. Each variant carries the offending value.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum PricingError {
    /// The named input is NaN or infinite.
    #[error("{field} must be finite, got {value}")]
    NonFinite {
        /// Name of the input.
        field: &'static str,
        /// The rejected value.
        value: f64,
    },
    /// Spot must be strictly positive.
    #[error("spot must be positive, got {0}")]
    InvalidSpot(f64),
    /// Strike must be strictly positive.
    #[error("strike must be positive, got {0}")]
    InvalidStrike(f64),
    /// Time to expiry must be zero or positive.
    #[error("time to expiry must not be negative, got {0}")]
    InvalidTimeToExpiry(f64),
    /// Volatility must be strictly positive.
    #[error("volatility must be positive, got {0}")]
    InvalidVolatility(f64),
    /// The rate is outside the sanity bound of -1.0 to 1.0.
    #[error("rate must lie within -1.0 to 1.0, got {0}")]
    RateOutOfRange(f64),
    /// The dividend yield is outside the sanity bound of -1.0 to 1.0.
    #[error("dividend yield must lie within -1.0 to 1.0, got {0}")]
    DividendYieldOutOfRange(f64),
}
