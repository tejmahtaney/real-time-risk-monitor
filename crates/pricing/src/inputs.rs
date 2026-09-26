//! Validated Black-Scholes-Merton inputs.

use crate::error::PricingError;

/// Bound on the absolute value of the rate and the dividend yield. A sanity check, not a model limit.
const RATE_BOUND: f64 = 1.0;

/// Whether an option is a call or a put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OptionType {
    /// The right to buy the underlying at the strike.
    Call,
    /// The right to sell the underlying at the strike.
    Put,
}

/// Validated inputs for pricing one European option.
///
/// The fields are private and the only way in is [`BsInputs::new`] (or the
/// `with_*` copies, which validate again), so a value that exists is valid.
/// For that reason it has no `Deserialize` implementation.
///
/// Rates, yields and volatilities are decimals, and rates are continuously
/// compounded. Time is in years.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BsInputs {
    spot: f64,
    strike: f64,
    time_to_expiry: f64,
    rate: f64,
    dividend_yield: f64,
    volatility: f64,
}

fn finite(field: &'static str, value: f64) -> Result<(), PricingError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(PricingError::NonFinite { field, value })
    }
}

impl BsInputs {
    /// Validates and builds the inputs.
    ///
    /// Rules: every field is finite; spot > 0; strike > 0; time to expiry ≥ 0;
    /// volatility > 0; rate and dividend yield lie within -1.0 to 1.0.
    ///
    /// # Errors
    ///
    /// Returns the [`PricingError`] variant for the first rule that fails.
    ///
    /// ```
    /// use pricing::{BsInputs, PricingError};
    ///
    /// assert!(BsInputs::new(100.0, 100.0, 1.0, 0.05, 0.0, 0.2).is_ok());
    /// assert_eq!(
    ///     BsInputs::new(-1.0, 100.0, 1.0, 0.05, 0.0, 0.2),
    ///     Err(PricingError::InvalidSpot(-1.0)),
    /// );
    /// ```
    pub fn new(
        spot: f64,
        strike: f64,
        time_to_expiry: f64,
        rate: f64,
        dividend_yield: f64,
        volatility: f64,
    ) -> Result<Self, PricingError> {
        finite("spot", spot)?;
        finite("strike", strike)?;
        finite("time_to_expiry", time_to_expiry)?;
        finite("rate", rate)?;
        finite("dividend_yield", dividend_yield)?;
        finite("volatility", volatility)?;
        if spot <= 0.0 {
            return Err(PricingError::InvalidSpot(spot));
        }
        if strike <= 0.0 {
            return Err(PricingError::InvalidStrike(strike));
        }
        if time_to_expiry < 0.0 {
            return Err(PricingError::InvalidTimeToExpiry(time_to_expiry));
        }
        if volatility <= 0.0 {
            return Err(PricingError::InvalidVolatility(volatility));
        }
        if !(-RATE_BOUND..=RATE_BOUND).contains(&rate) {
            return Err(PricingError::RateOutOfRange(rate));
        }
        if !(-RATE_BOUND..=RATE_BOUND).contains(&dividend_yield) {
            return Err(PricingError::DividendYieldOutOfRange(dividend_yield));
        }
        Ok(Self {
            spot,
            strike,
            time_to_expiry,
            rate,
            dividend_yield,
            volatility,
        })
    }

    /// Spot price of the underlying.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.spot(), 100.0);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn spot(&self) -> f64 {
        self.spot
    }

    /// Strike price.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.strike(), 90.0);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn strike(&self) -> f64 {
        self.strike
    }

    /// Time to expiry in years.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 0.5, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.time_to_expiry(), 0.5);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn time_to_expiry(&self) -> f64 {
        self.time_to_expiry
    }

    /// Continuously compounded risk-free rate as a decimal.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.rate(), 0.05);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Continuous dividend yield as a decimal.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.02, 0.2)?;
    /// assert_eq!(i.dividend_yield(), 0.02);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn dividend_yield(&self) -> f64 {
        self.dividend_yield
    }

    /// Annualised volatility as a decimal.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.volatility(), 0.2);
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn volatility(&self) -> f64 {
        self.volatility
    }

    /// Returns a validated copy with a different spot.
    ///
    /// # Errors
    ///
    /// Fails if the new spot breaks a validation rule.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.with_spot(101.0)?.spot(), 101.0);
    /// assert!(i.with_spot(f64::NAN).is_err());
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn with_spot(&self, spot: f64) -> Result<Self, PricingError> {
        Self::new(
            spot,
            self.strike,
            self.time_to_expiry,
            self.rate,
            self.dividend_yield,
            self.volatility,
        )
    }

    /// Returns a validated copy with a different volatility.
    ///
    /// # Errors
    ///
    /// Fails if the new volatility breaks a validation rule.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.with_volatility(0.3)?.volatility(), 0.3);
    /// assert!(i.with_volatility(0.0).is_err());
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn with_volatility(&self, volatility: f64) -> Result<Self, PricingError> {
        Self::new(
            self.spot,
            self.strike,
            self.time_to_expiry,
            self.rate,
            self.dividend_yield,
            volatility,
        )
    }

    /// Returns a validated copy with a different time to expiry, in years.
    ///
    /// # Errors
    ///
    /// Fails if the new time breaks a validation rule.
    ///
    /// ```
    /// # use pricing::BsInputs;
    /// let i = BsInputs::new(100.0, 90.0, 1.0, 0.05, 0.0, 0.2)?;
    /// assert_eq!(i.with_time_to_expiry(0.25)?.time_to_expiry(), 0.25);
    /// assert!(i.with_time_to_expiry(-0.1).is_err());
    /// # Ok::<(), pricing::PricingError>(())
    /// ```
    pub fn with_time_to_expiry(&self, time_to_expiry: f64) -> Result<Self, PricingError> {
        Self::new(
            self.spot,
            self.strike,
            time_to_expiry,
            self.rate,
            self.dividend_yield,
            self.volatility,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn ok() -> [f64; 6] {
        [100.0, 100.0, 1.0, 0.05, 0.02, 0.2]
    }

    fn build(a: [f64; 6]) -> Result<BsInputs, PricingError> {
        BsInputs::new(a[0], a[1], a[2], a[3], a[4], a[5])
    }

    #[test]
    fn valid_inputs_round_trip_through_getters() {
        let i = build(ok()).unwrap();
        let got = [
            i.spot(),
            i.strike(),
            i.time_to_expiry(),
            i.rate(),
            i.dividend_yield(),
            i.volatility(),
        ];
        for (g, e) in got.into_iter().zip(ok()) {
            assert_abs_diff_eq!(g, e, epsilon = 1e-15);
        }
    }

    #[test]
    fn zero_time_to_expiry_is_allowed() {
        assert!(build([100.0, 100.0, 0.0, 0.05, 0.0, 0.2]).is_ok());
    }

    #[test]
    fn rate_bounds_are_inclusive() {
        assert!(build([100.0, 100.0, 1.0, 1.0, -1.0, 0.2]).is_ok());
        assert!(build([100.0, 100.0, 1.0, -1.0, 1.0, 0.2]).is_ok());
    }

    #[test]
    fn non_finite_values_are_rejected_for_every_field() {
        let names = [
            "spot",
            "strike",
            "time_to_expiry",
            "rate",
            "dividend_yield",
            "volatility",
        ];
        for (idx, name) in names.iter().enumerate() {
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut a = ok();
                a[idx] = bad;
                let err = build(a).unwrap_err();
                assert!(
                    matches!(err, PricingError::NonFinite { field, value }
                        if field == *name && value.to_bits() == bad.to_bits()),
                    "{name}={bad}: got {err:?}"
                );
            }
        }
    }

    #[test]
    fn spot_must_be_positive() {
        for bad in [0.0, -5.0] {
            let mut a = ok();
            a[0] = bad;
            assert_eq!(build(a), Err(PricingError::InvalidSpot(bad)));
        }
    }

    #[test]
    fn strike_must_be_positive() {
        for bad in [0.0, -5.0] {
            let mut a = ok();
            a[1] = bad;
            assert_eq!(build(a), Err(PricingError::InvalidStrike(bad)));
        }
    }

    #[test]
    fn time_to_expiry_must_not_be_negative() {
        let mut a = ok();
        a[2] = -1e-9;
        assert_eq!(build(a), Err(PricingError::InvalidTimeToExpiry(-1e-9)));
    }

    #[test]
    fn volatility_must_be_positive() {
        for bad in [0.0, -0.2] {
            let mut a = ok();
            a[5] = bad;
            assert_eq!(build(a), Err(PricingError::InvalidVolatility(bad)));
        }
    }

    #[test]
    fn rate_must_be_within_bound() {
        for bad in [1.0001, -1.5] {
            let mut a = ok();
            a[3] = bad;
            assert_eq!(build(a), Err(PricingError::RateOutOfRange(bad)));
        }
    }

    #[test]
    fn dividend_yield_must_be_within_bound() {
        for bad in [1.0001, -1.5] {
            let mut a = ok();
            a[4] = bad;
            assert_eq!(build(a), Err(PricingError::DividendYieldOutOfRange(bad)));
        }
    }

    #[test]
    fn with_methods_change_one_field_and_revalidate() {
        let i = build(ok()).unwrap();
        let s = i.with_spot(90.0).unwrap();
        assert_abs_diff_eq!(s.spot(), 90.0, epsilon = 1e-15);
        assert_abs_diff_eq!(s.strike(), 100.0, epsilon = 1e-15);
        assert_abs_diff_eq!(s.volatility(), 0.2, epsilon = 1e-15);
        let v = i.with_volatility(0.4).unwrap();
        assert_abs_diff_eq!(v.spot(), 100.0, epsilon = 1e-15);
        assert_abs_diff_eq!(v.volatility(), 0.4, epsilon = 1e-15);
        let t = i.with_time_to_expiry(0.0).unwrap();
        assert_abs_diff_eq!(t.time_to_expiry(), 0.0, epsilon = 1e-15);
        assert_eq!(i.with_spot(-1.0), Err(PricingError::InvalidSpot(-1.0)));
        assert_eq!(
            i.with_volatility(0.0),
            Err(PricingError::InvalidVolatility(0.0))
        );
        assert_eq!(
            i.with_time_to_expiry(-1.0),
            Err(PricingError::InvalidTimeToExpiry(-1.0))
        );
    }
}
