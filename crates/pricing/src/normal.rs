//! The standard normal distribution.

use std::f64::consts::FRAC_1_SQRT_2;

/// 1 / √(2π), the height of the density at zero.
const FRAC_1_SQRT_2PI: f64 = 0.398_942_280_401_432_7;

/// Probability density of the standard normal distribution.
///
/// φ(x) = exp(−x²/2) / √(2π)
///
/// ```
/// use pricing::normal;
///
/// let peak = 1.0 / (2.0 * std::f64::consts::PI).sqrt();
/// assert!((normal::pdf(0.0) - peak).abs() < 1e-15);
/// assert_eq!(normal::pdf(1.5), normal::pdf(-1.5));
/// ```
pub fn pdf(x: f64) -> f64 {
    FRAC_1_SQRT_2PI * (-0.5 * x * x).exp()
}

/// Cumulative distribution function of the standard normal distribution.
///
/// N(x) = ½ · erfc(−x / √2)
///
/// The complementary error function is used instead of `0.5 * (1 + erf(x / √2))`
/// or `1 − N(−x)` because those forms cancel. In the lower tail N(x) is tiny
/// (N(−10) ≈ 7.6e-24), and `1 + erf(...)` is a sum of two numbers that agree
/// almost to the last bit, so every significant digit is lost. `erfc` is
/// evaluated directly in the tail and keeps full relative precision. Deep
/// out-of-the-money option prices are products of exactly these tail values.
///
/// ```
/// use pricing::normal;
///
/// assert_eq!(normal::cdf(0.0), 0.5);
/// let tail = normal::cdf(-10.0);
/// assert!((tail / 7.6198530241605261e-24 - 1.0).abs() < 1e-12);
/// ```
pub fn cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x * FRAC_1_SQRT_2)
}

#[cfg(test)]
mod tests {
    // Reference values are pasted verbatim from mpmath, so keep every digit.
    #![allow(clippy::excessive_precision, clippy::unreadable_literal)]
    use super::*;
    use approx::{assert_abs_diff_eq, assert_relative_eq};
    use std::f64::consts::PI;

    #[test]
    fn cdf_matches_mpmath_references() {
        let cases = [
            (0.0, 0.5),
            (1.0, 0.84134474606854295),
            (-1.0, 0.15865525393145705),
            (1.96, 0.97500210485177956),
            (-5.0, 2.8665157187919391e-7),
            (-10.0, 7.6198530241605261e-24),
            (-20.0, 2.7536241186062337e-89),
        ];
        for (x, expected) in cases {
            assert_relative_eq!(cdf(x), expected, max_relative = 1e-12);
        }
    }

    #[test]
    fn cdf_is_complementary() {
        for x in [0.3, 1.0, 2.5, 4.0] {
            assert_relative_eq!(cdf(x) + cdf(-x), 1.0, epsilon = 1e-15);
        }
    }

    #[test]
    fn pdf_is_symmetric() {
        for x in [0.1, 0.5, 1.0, 3.0, 8.0] {
            assert_abs_diff_eq!(pdf(x), pdf(-x), epsilon = 1e-15);
        }
    }

    #[test]
    fn pdf_at_zero_is_one_over_root_two_pi() {
        assert_relative_eq!(pdf(0.0), 1.0 / (2.0 * PI).sqrt(), max_relative = 1e-15);
    }
}
