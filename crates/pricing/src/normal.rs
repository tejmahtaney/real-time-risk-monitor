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

/// Wichura's AS241 rational-approximation coefficients (Applied Statistics 37, 1988),
/// reproduced verbatim from the published algorithm — the same ones R's `qnorm` uses.
/// Every digit matters for [`inv_cdf`]'s accuracy, so `excessive_precision` and
/// `unreadable_literal` are allowed for this whole module rather than truncated to
/// clippy's preferred precision.
#[allow(
    clippy::excessive_precision,
    clippy::unreadable_literal,
    clippy::inconsistent_digit_grouping
)]
mod as241 {
    /// Central-region rational approximation coefficients for [`super::inv_cdf`]
    /// (|p − 0.5| ≤ 0.425), highest degree first.
    pub(super) const CENTRAL_NUM: [f64; 8] = [
        2509.080_928_730_122_672_7,
        33430.575_583_588_128_105,
        67265.770_927_008_700_853,
        45921.953_931_549_871_457,
        13731.693_765_509_461_125,
        1971.590_950_306_551_442_7,
        133.141_667_891_784_377_45,
        3.387_132_872_796_366_608,
    ];
    pub(super) const CENTRAL_DEN: [f64; 8] = [
        5226.495_278_852_854_561,
        28729.085_735_721_942_674,
        39307.895_800_092_710_61,
        21213.794_301_586_595_867,
        5394.196_021_424_751_107_7,
        687.187_007_492_057_908_3,
        42.313_330_701_600_911_252,
        1.0,
    ];

    /// Moderate-tail rational approximation coefficients for [`super::inv_cdf`]
    /// (`sqrt(-ln(min(p, 1-p)))` in `(0, 5]`), highest degree first.
    pub(super) const TAIL_NUM: [f64; 8] = [
        7.745_450_142_783_414_076_4e-4,
        0.022_723_844_989_269_184_583,
        0.241_780_725_177_450_611_77,
        1.270_458_252_452_368_382_58,
        3.647_848_324_763_204_605_04,
        5.769_497_221_460_691_405_5,
        4.630_337_846_156_545_295_9,
        1.423_437_110_749_683_577_34,
    ];
    pub(super) const TAIL_DEN: [f64; 8] = [
        1.050_750_071_644_416_843_24e-9,
        5.475_938_084_995_344_946e-4,
        0.015_198_666_563_616_457_196_6,
        0.148_103_976_427_480_074_59,
        0.689_767_334_985_100_004_55,
        1.676_384_830_183_803_849_4,
        2.053_191_626_637_758_821_87,
        1.0,
    ];

    /// Extreme-tail rational approximation coefficients for [`super::inv_cdf`]
    /// (`sqrt(-ln(min(p, 1-p)))` beyond 5), highest degree first.
    pub(super) const EXTREME_NUM: [f64; 8] = [
        2.010_334_399_292_288_132_65e-7,
        2.711_555_568_743_487_578_15e-5,
        0.001_242_660_947_388_078_438_6,
        0.026_532_189_526_576_123_093,
        0.296_560_571_828_504_891_23,
        1.784_826_539_917_291_335_8,
        5.463_784_911_164_114_369_9,
        6.657_904_643_501_103_777_2,
    ];
    pub(super) const EXTREME_DEN: [f64; 8] = [
        2.044_263_103_389_939_785_64e-15,
        1.421_511_758_316_445_888_7e-7,
        1.846_318_317_510_054_681_8e-5,
        7.868_691_311_456_132_591e-4,
        0.014_875_361_290_850_614_852_5,
        0.136_929_880_922_735_805_31,
        0.599_832_206_555_887_937_69,
        1.0,
    ];
}
use as241::{CENTRAL_DEN, CENTRAL_NUM, EXTREME_DEN, EXTREME_NUM, TAIL_DEN, TAIL_NUM};

/// Evaluates a polynomial by Horner's method from its coefficients, highest
/// degree first. Used only by [`inv_cdf`]; no allocation, `coeffs` is a
/// borrowed fixed-size array.
fn horner(coeffs: &[f64], x: f64) -> f64 {
    coeffs.iter().fold(0.0, |acc, &c| acc.mul_add(x, c))
}

/// The inverse of [`cdf`]: the standard normal quantile function, Φ⁻¹(p).
///
/// Implements Wichura's AS241 algorithm (Applied Statistics 37, 1988), the
/// rational-minimax approximation behind R's `qnorm` and most other
/// statistics libraries. It is accurate to full `f64` precision across the
/// whole open interval (0, 1) via three regimes: a central rational
/// approximation for `|p − 0.5| ≤ 0.425`, and two further rational
/// approximations in the tails (selected by how far `sqrt(-ln(min(p, 1-p)))`
/// falls into the tail, split at 5.0).
///
/// `p ≤ 0.0` returns `f64::NEG_INFINITY`; `p ≥ 1.0` returns `f64::INFINITY` —
/// the correct limiting values, not a panic or `NaN`. Like [`pdf`] and
/// [`cdf`], this function is deliberately infallible.
///
/// ```
/// use pricing::normal::inv_cdf;
///
/// assert!((inv_cdf(0.95) - 1.6448536269514723).abs() < 1e-12);
/// assert!((inv_cdf(0.975) - 1.9599639845400539).abs() < 1e-12);
/// assert!((inv_cdf(0.99) - 2.3263478740408408).abs() < 1e-12);
/// ```
pub fn inv_cdf(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }

    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180_625 - q * q;
        return q * horner(&CENTRAL_NUM, r) / horner(&CENTRAL_DEN, r);
    }

    let tail_p = if q < 0.0 { p } else { 1.0 - p };
    let r = (-tail_p.ln()).sqrt();

    let val = if r <= 5.0 {
        let r = r - 1.6;
        horner(&TAIL_NUM, r) / horner(&TAIL_DEN, r)
    } else {
        let r = r - 5.0;
        horner(&EXTREME_NUM, r) / horner(&EXTREME_DEN, r)
    };

    if q < 0.0 { -val } else { val }
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

    #[test]
    fn inv_cdf_matches_reference_quantiles() {
        let cases = [
            (0.95, 1.6448536269514723),
            (0.975, 1.9599639845400539),
            (0.99, 2.3263478740408408),
        ];
        for (p, expected) in cases {
            assert_relative_eq!(inv_cdf(p), expected, max_relative = 1e-12);
        }
    }

    #[test]
    fn inv_cdf_is_antisymmetric_about_one_half() {
        for p in [0.55, 0.7, 0.9, 0.99, 0.999] {
            assert_relative_eq!(inv_cdf(p), -inv_cdf(1.0 - p), max_relative = 1e-12);
        }
    }

    // These two tests check exact sentinel outputs (an exact zero from q = 0, and the
    // infinities `inv_cdf` returns outside (0, 1)) rather than a general float
    // computation, so an exact comparison is the correct check here, not just a
    // convenient one — `approx`'s relative comparison is undefined against zero and
    // against infinity.
    #[allow(clippy::float_cmp)]
    #[test]
    fn inv_cdf_at_one_half_is_zero() {
        assert_eq!(inv_cdf(0.5), 0.0);
    }

    #[allow(clippy::float_cmp)]
    #[test]
    fn inv_cdf_at_the_open_interval_boundary_is_infinite() {
        assert_eq!(inv_cdf(0.0), f64::NEG_INFINITY);
        assert_eq!(inv_cdf(-1.0), f64::NEG_INFINITY);
        assert_eq!(inv_cdf(1.0), f64::INFINITY);
        assert_eq!(inv_cdf(2.0), f64::INFINITY);
    }

    #[test]
    fn cdf_of_inv_cdf_round_trips_across_the_open_interval() {
        let ps = [
            1e-10,
            1e-8,
            1e-6,
            1e-4,
            0.001,
            0.01,
            0.1,
            0.3,
            0.5,
            0.7,
            0.9,
            0.99,
            0.999,
            1.0 - 1e-4,
            1.0 - 1e-6,
            1.0 - 1e-8,
            1.0 - 1e-10,
        ];
        for p in ps {
            assert_relative_eq!(cdf(inv_cdf(p)), p, max_relative = 1e-13);
        }
    }
}
