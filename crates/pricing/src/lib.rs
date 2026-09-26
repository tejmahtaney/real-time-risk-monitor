//! Pure option-pricing maths: normal distribution, Black-Scholes-Merton price, Greeks and implied volatility. No I/O, no async and no internal dependencies.
//!
//! Part 2 provides the [`normal`] distribution, validated [`BsInputs`] and the
//! Black-Scholes-Merton [`price`] of a European option with a continuous
//! dividend yield. Part 3 adds [`greeks`], the analytic delta, gamma, vega,
//! theta, rho, vanna and volga. All rates, yields and volatilities are decimals
//! (0.05 means 5 %) and time is in years.
//!
//! ```
//! use pricing::{price, BsInputs, OptionType};
//!
//! // Hull's example: S = 42, K = 40, T = 0.5, r = 10 %, q = 0, sigma = 20 %.
//! let inputs = BsInputs::new(42.0, 40.0, 0.5, 0.10, 0.0, 0.20)?;
//! let call = price(OptionType::Call, &inputs);
//! assert!((call - 4.76).abs() < 0.005);
//! # Ok::<(), pricing::PricingError>(())
//! ```

#![warn(missing_docs)]

mod bsm;
mod error;
mod greeks;
mod inputs;
pub mod normal;

pub use bsm::{MIN_TOTAL_VOL, price};
pub use error::PricingError;
pub use greeks::{Greeks, greeks};
pub use inputs::{BsInputs, OptionType};
