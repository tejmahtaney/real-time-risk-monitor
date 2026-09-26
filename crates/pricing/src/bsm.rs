//! Black-Scholes-Merton price of a European option with a continuous dividend yield.

use crate::inputs::{BsInputs, OptionType};
use crate::normal;

/// Below this value of σ·√T the option is treated as deterministic.
pub const MIN_TOTAL_VOL: f64 = 1e-12;

/// The quantities every BSM formula shares, computed once per option.
///
/// Part 3 (Greeks) reuses this so that d1, d2, N(·) and the discount factors
/// are never recomputed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Terms {
    /// e^(−rT).
    pub(crate) discount_rate: f64,
    /// e^(−qT).
    pub(crate) discount_yield: f64,
    /// √T.
    // Not read by `price`. Part 3 (Greeks) needs √T for vega and gamma, and the
    // helper is specified to compute it once, so it lives here.
    #[allow(dead_code)]
    pub(crate) sqrt_t: f64,
    /// `(d1, d2)`, or `None` when σ·√T is below [`MIN_TOTAL_VOL`] and they are
    /// not defined in floating point.
    pub(crate) d: Option<(f64, f64)>,
}

impl Terms {
    /// d1 = [ln(S/K) + (r − q + σ²/2)·T] / (σ·√T) and d2 = d1 − σ·√T.
    pub(crate) fn new(inputs: &BsInputs) -> Self {
        let t = inputs.time_to_expiry();
        let sqrt_t = t.sqrt();
        let total_vol = inputs.volatility() * sqrt_t;
        let d = if total_vol < MIN_TOTAL_VOL {
            None
        } else {
            let sigma = inputs.volatility();
            let drift = inputs.rate() - inputs.dividend_yield() + 0.5 * sigma * sigma;
            let d1 = ((inputs.spot() / inputs.strike()).ln() + drift * t) / total_vol;
            Some((d1, d1 - total_vol))
        };
        Self {
            discount_rate: (-inputs.rate() * t).exp(),
            discount_yield: (-inputs.dividend_yield() * t).exp(),
            sqrt_t,
            d,
        }
    }
}

/// Black-Scholes-Merton price of a European option with continuous dividend yield `q`.
///
/// - d1 = [ln(S/K) + (r − q + σ²/2)·T] / (σ·√T)
/// - d2 = d1 − σ·√T
/// - Call = S·e^(−qT)·N(d1) − K·e^(−rT)·N(d2)
/// - Put = K·e^(−rT)·N(−d2) − S·e^(−qT)·N(−d1)
///
/// N is the standard normal CDF, see [`normal::cdf`]. It cannot fail because a
/// [`BsInputs`] is always valid.
///
/// Degenerate cases:
///
/// - T = 0 returns intrinsic value, max(S − K, 0) for a call and
///   max(K − S, 0) for a put.
/// - If σ·√T is below [`MIN_TOTAL_VOL`] the result is the discounted forward
///   intrinsic value, max(S·e^(−qT) − K·e^(−rT), 0) for a call and
///   max(K·e^(−rT) − S·e^(−qT), 0) for a put.
///
/// ```
/// use pricing::{price, BsInputs, OptionType};
///
/// let inputs = BsInputs::new(42.0, 40.0, 0.5, 0.10, 0.0, 0.20)?;
/// let call = price(OptionType::Call, &inputs);
/// let put = price(OptionType::Put, &inputs);
/// assert!((call - 4.7594223928715334).abs() < 1e-10);
/// assert!((put - 0.80859937290009365).abs() < 1e-10);
/// # Ok::<(), pricing::PricingError>(())
/// ```
pub fn price(option_type: OptionType, inputs: &BsInputs) -> f64 {
    let spot = inputs.spot();
    let strike = inputs.strike();

    if inputs.time_to_expiry() == 0.0 {
        return match option_type {
            OptionType::Call => (spot - strike).max(0.0),
            OptionType::Put => (strike - spot).max(0.0),
        };
    }

    let terms = Terms::new(inputs);
    let forward_spot = spot * terms.discount_yield;
    let discounted_strike = strike * terms.discount_rate;

    let Some((d1, d2)) = terms.d else {
        // As σ → 0 the terminal price S_T = S·e^((r−q−σ²/2)T + σ√T·Z) tends to
        // the forward S·e^((r−q)T) with certainty. The option then pays
        // max(F − K, 0) at expiry, and discounting by e^(−rT) gives
        // max(S·e^(−qT) − K·e^(−rT), 0). N(d1) and N(d2) tend to 0 or 1, so
        // the closed form converges to the same value, but d1 and d2 blow up
        // as σ·√T → 0, so we return the limit directly.
        return match option_type {
            OptionType::Call => (forward_spot - discounted_strike).max(0.0),
            OptionType::Put => (discounted_strike - forward_spot).max(0.0),
        };
    };

    let value = match option_type {
        OptionType::Call => forward_spot * normal::cdf(d1) - discounted_strike * normal::cdf(d2),
        OptionType::Put => discounted_strike * normal::cdf(-d2) - forward_spot * normal::cdf(-d1),
    };
    // The exact price is never negative. Deep out of the money the two terms
    // are almost equal and rounding can leave a residue of about -1e-30.
    value.max(0.0)
}

#[cfg(test)]
mod tests {
    // Reference values are pasted verbatim from mpmath, so keep every digit.
    #![allow(clippy::excessive_precision, clippy::unreadable_literal)]
    use super::*;
    use approx::assert_abs_diff_eq;
    use proptest::prelude::*;

    fn inputs(s: f64, k: f64, t: f64, r: f64, q: f64, v: f64) -> BsInputs {
        BsInputs::new(s, k, t, r, q, v).unwrap()
    }

    fn check(i: &BsInputs, call: f64, put: f64) {
        assert_abs_diff_eq!(price(OptionType::Call, i), call, epsilon = 1e-10);
        assert_abs_diff_eq!(price(OptionType::Put, i), put, epsilon = 1e-10);
    }

    #[test]
    fn hull_example() {
        let i = inputs(42.0, 40.0, 0.5, 0.10, 0.0, 0.20);
        check(&i, 4.7594223928715334, 0.80859937290009365);
        // Hull's book rounds to 4.76 and 0.81.
        assert_abs_diff_eq!(price(OptionType::Call, &i), 4.76, epsilon = 0.005);
        assert_abs_diff_eq!(price(OptionType::Put, &i), 0.81, epsilon = 0.005);
    }

    #[test]
    fn dividend_case() {
        let i = inputs(100.0, 95.0, 0.75, 0.05, 0.02, 0.25);
        check(&i, 12.163047711528401, 5.1553234347002025);
    }

    #[test]
    fn haug_generalised_bsm_example() {
        let i = inputs(75.0, 70.0, 0.5, 0.10, 0.05, 0.35);
        check(&i, 10.649137515710323, 4.0869538286353533);
        // Haug's book prints 4.0870 for the put.
        assert_abs_diff_eq!(price(OptionType::Put, &i), 4.0870, epsilon = 5e-5);
    }

    #[test]
    fn zero_time_gives_intrinsic_value() {
        let itm_call = inputs(110.0, 100.0, 0.0, 0.05, 0.02, 0.2);
        assert_abs_diff_eq!(price(OptionType::Call, &itm_call), 10.0, epsilon = 1e-12);
        assert_abs_diff_eq!(price(OptionType::Put, &itm_call), 0.0, epsilon = 1e-12);
        let itm_put = inputs(90.0, 100.0, 0.0, 0.05, 0.02, 0.2);
        assert_abs_diff_eq!(price(OptionType::Call, &itm_put), 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(price(OptionType::Put, &itm_put), 10.0, epsilon = 1e-12);
    }

    #[test]
    fn tiny_vol_gives_discounted_forward_intrinsic_value() {
        let (s, k, t, r, q) = (100.0, 95.0, 1.0, 0.05, 0.02);
        let i = inputs(s, k, t, r, q, 1e-9);
        let fwd = s * (-q * t).exp();
        let strike = k * (-r * t).exp();
        assert_abs_diff_eq!(
            price(OptionType::Call, &i),
            (fwd - strike).max(0.0),
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(
            price(OptionType::Put, &i),
            (strike - fwd).max(0.0),
            epsilon = 1e-12
        );

        // A put that is out of the money on the forward.
        let i = inputs(100.0, 80.0, 1.0, 0.05, 0.0, 1e-9);
        assert_abs_diff_eq!(price(OptionType::Put, &i), 0.0, epsilon = 1e-12);
    }

    #[test]
    fn vol_below_threshold_matches_limit_of_closed_form() {
        // Just above the threshold the closed form must agree with the limit.
        let (s, k, t, r, q) = (100.0, 95.0, 1.0, 0.05, 0.02);
        let above = inputs(s, k, t, r, q, 2e-12);
        let below = inputs(s, k, t, r, q, 5e-13);
        assert_abs_diff_eq!(
            price(OptionType::Call, &above),
            price(OptionType::Call, &below),
            epsilon = 1e-9
        );
    }

    #[test]
    fn extreme_moneyness_is_finite_and_non_negative() {
        for (s, k) in [(100.0, 1.0), (1.0, 100.0)] {
            let i = inputs(s, k, 0.5, 0.03, 0.01, 0.25);
            for ty in [OptionType::Call, OptionType::Put] {
                let p = price(ty, &i);
                assert!(p.is_finite() && p >= 0.0, "{ty:?} S={s} K={k}: {p}");
            }
        }
        // Deep OTM keeps relative precision rather than collapsing to zero.
        let otm_call = price(OptionType::Call, &inputs(1.0, 100.0, 0.5, 0.03, 0.01, 0.25));
        assert!(otm_call > 0.0 && otm_call < 1e-30);
    }

    #[test]
    fn monotone_in_spot_vol_and_strike() {
        let base = |s: f64, k: f64, v: f64| inputs(s, k, 1.0, 0.03, 0.01, v);
        let grid = |lo: f64, step: f64| (0..20).map(move |n| lo + step * f64::from(n));

        let pairs =
            |f: &dyn Fn(f64) -> BsInputs| -> Vec<BsInputs> { grid(0.0, 1.0).map(f).collect() };

        let by_spot = pairs(&|n| base(70.0 + 3.0 * n, 100.0, 0.25));
        let by_vol = pairs(&|n| base(100.0, 100.0, 0.05 + 0.03 * n));
        let by_strike = pairs(&|n| base(100.0, 70.0 + 3.0 * n, 0.25));

        let increasing =
            |v: &[BsInputs], ty| v.windows(2).all(|w| price(ty, &w[1]) > price(ty, &w[0]));
        let decreasing =
            |v: &[BsInputs], ty| v.windows(2).all(|w| price(ty, &w[1]) < price(ty, &w[0]));

        assert!(increasing(&by_spot, OptionType::Call));
        assert!(decreasing(&by_spot, OptionType::Put));
        assert!(increasing(&by_vol, OptionType::Call));
        assert!(increasing(&by_vol, OptionType::Put));
        assert!(decreasing(&by_strike, OptionType::Call));
        assert!(increasing(&by_strike, OptionType::Put));
    }

    prop_compose! {
        fn domain()(
            s in 1.0..1000.0f64,
            k_factor in 0.2..5.0f64,
            t in (1.0 / 365.0)..5.0f64,
            r in -0.02..0.15f64,
            q in 0.0..0.08f64,
            v in 0.01..2.0f64,
        ) -> (f64, f64, f64, f64, f64, f64) {
            (s, s * k_factor, t, r, q, v)
        }
    }

    proptest! {
        #[test]
        fn put_call_parity((s, k, t, r, q, v) in domain()) {
            let i = inputs(s, k, t, r, q, v);
            let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
            let rhs = s * (-q * t).exp() - k * (-r * t).exp();
            prop_assert!((lhs - rhs).abs() <= 1e-9 * s.max(k), "lhs={lhs} rhs={rhs}");
        }

        #[test]
        fn no_arbitrage_bounds((s, k, t, r, q, v) in domain()) {
            let i = inputs(s, k, t, r, q, v);
            let tol = 1e-9 * s.max(k);
            let fwd = s * (-q * t).exp();
            let strike = k * (-r * t).exp();
            let call = price(OptionType::Call, &i);
            let put = price(OptionType::Put, &i);
            prop_assert!(call >= (fwd - strike).max(0.0) - tol && call <= fwd + tol);
            prop_assert!(put >= (strike - fwd).max(0.0) - tol && put <= strike + tol);
        }
    }
}
