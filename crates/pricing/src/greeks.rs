//! Analytic Black-Scholes-Merton Greeks with a continuous dividend yield.

use crate::bsm::{Terms, intrinsic};
use crate::inputs::{BsInputs, OptionType};
use crate::normal;

/// Price and raw per-unit sensitivities of one European option.
///
/// These are the pure-maths units, with no quantity or contract multiplier.
/// Reporting units (per day, per vol point, per position) start in the
/// `portfolio` crate.
///
/// In every formula below φ is the standard normal pdf, N the standard normal
/// cdf, and d1 and d2 are as in [`price`](crate::price).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Greeks {
    /// Black-Scholes-Merton price, identical to [`price`](crate::price).
    pub price: f64,
    /// ∂V/∂S, the change in value per unit move in spot.
    ///
    /// - Call: e^(−qT)·N(d1)
    /// - Put: −e^(−qT)·N(−d1)
    pub delta: f64,
    /// ∂²V/∂S², the change in delta per unit move in spot. Same for call and put.
    ///
    /// e^(−qT)·φ(d1) / (S·σ·√T)
    pub gamma: f64,
    /// ∂V/∂σ per 1.00 of volatility. Same for call and put.
    ///
    /// S·e^(−qT)·φ(d1)·√T
    pub vega: f64,
    /// ∂V/∂t per year of calendar time passing, which is −∂V/∂T.
    ///
    /// - Call: −S·e^(−qT)·φ(d1)·σ / (2√T) − r·K·e^(−rT)·N(d2) + q·S·e^(−qT)·N(d1)
    /// - Put: −S·e^(−qT)·φ(d1)·σ / (2√T) + r·K·e^(−rT)·N(−d2) − q·S·e^(−qT)·N(−d1)
    pub theta: f64,
    /// ∂V/∂r per 1.00 of the continuously compounded rate.
    ///
    /// - Call: K·T·e^(−rT)·N(d2)
    /// - Put: −K·T·e^(−rT)·N(−d2)
    pub rho: f64,
    /// ∂²V/∂S∂σ, the change in delta per 1.00 of volatility. Same for call and put.
    ///
    /// −e^(−qT)·φ(d1)·d2 / σ
    pub vanna: f64,
    /// ∂²V/∂σ², the change in vega per 1.00 of volatility. Same for call and put.
    ///
    /// vega·d1·d2 / σ
    pub volga: f64,
}

/// Price and analytic Greeks of a European option, from one shared set of terms.
///
/// d1, d2, √T, the discount factors, the two normal CDFs and φ(d1) are computed
/// once and every output is derived from them. The price comes from the same
/// helper as [`price`](crate::price), so the two never disagree. See the fields
/// of [`Greeks`] for each formula and its units.
///
/// The put delta is written as −e^(−qT)·N(−d1) and not as
/// e^(−qT)·(N(d1) − 1). For a deep out-of-the-money put N(d1) is within rounding
/// of 1, so the subtraction cancels every significant digit, while N(−d1) is
/// evaluated directly in the tail and keeps its relative precision.
///
/// Degenerate cases, all documented choices:
///
/// - **T = 0.** The price is intrinsic value. Call delta is 1 when S > K, 0 when
///   S < K and 0.5 when S = K, and put delta is call delta − 1. Every other
///   Greek is 0.
/// - **σ·√T below [`MIN_TOTAL_VOL`](crate::MIN_TOTAL_VOL).** The value is the
///   discounted forward intrinsic value V = ω·(S·e^(−qT) − K·e^(−rT)) when that
///   is positive, with ω = +1 for a call and −1 for a put. When it is strictly
///   positive (in the money) the Greeks are the derivatives of that
///   expression: delta = ω·e^(−qT), theta = −∂V/∂T = ω·(q·S·e^(−qT) −
///   r·K·e^(−rT)) and rho = ω·K·T·e^(−rT). Otherwise, including exactly at the
///   forward, they are 0. Gamma, vega, vanna and volga are 0 in this regime
///   because the value no longer depends on spot curvature or volatility.
///
/// ```
/// use pricing::{greeks, BsInputs, OptionType};
///
/// let inputs = BsInputs::new(42.0, 40.0, 0.5, 0.10, 0.0, 0.20)?;
/// let g = greeks(OptionType::Call, &inputs);
/// assert!((g.delta - 0.77913129094266894).abs() < 1e-10);
/// assert!((g.gamma - 0.049962670405911853).abs() < 1e-10);
/// # Ok::<(), pricing::PricingError>(())
/// ```
pub fn greeks(option_type: OptionType, inputs: &BsInputs) -> Greeks {
    let spot = inputs.spot();
    let strike = inputs.strike();

    if inputs.time_to_expiry() == 0.0 {
        let call_delta = if spot > strike {
            1.0
        } else if spot < strike {
            0.0
        } else {
            0.5
        };
        return Greeks {
            price: intrinsic(option_type, spot, strike),
            delta: match option_type {
                OptionType::Call => call_delta,
                OptionType::Put => call_delta - 1.0,
            },
            ..Greeks::ZERO
        };
    }

    let terms = Terms::new(inputs);
    let w = option_type.sign();
    let t = inputs.time_to_expiry();

    let Some((d1, d2)) = terms.d else {
        let price = terms.forward_intrinsic(option_type);
        if price <= 0.0 {
            return Greeks::ZERO;
        }
        let rate = inputs.rate();
        let yield_ = inputs.dividend_yield();
        return Greeks {
            price,
            delta: w * terms.discount_yield,
            theta: w * (yield_ * terms.forward_spot - rate * terms.discounted_strike),
            rho: w * terms.discounted_strike * t,
            ..Greeks::ZERO
        };
    };

    let sigma = inputs.volatility();
    // Signed CDFs: N(d) for a call and N(−d) for a put. Each is evaluated
    // directly, never as 1 − N(·), so the tails keep their precision.
    let n1 = normal::cdf(w * d1);
    let n2 = normal::cdf(w * d2);
    let pdf_d1 = normal::pdf(d1);
    let forward_pdf = terms.forward_spot * pdf_d1;
    let vega = forward_pdf * terms.sqrt_t;

    Greeks {
        price: terms.value(option_type, n1, n2),
        delta: w * terms.discount_yield * n1,
        gamma: terms.discount_yield * pdf_d1 / (spot * sigma * terms.sqrt_t),
        vega,
        theta: -forward_pdf * sigma / (2.0 * terms.sqrt_t)
            + w * (inputs.dividend_yield() * terms.forward_spot * n1
                - inputs.rate() * terms.discounted_strike * n2),
        rho: w * strike * t * terms.discount_rate * n2,
        vanna: -terms.discount_yield * pdf_d1 * d2 / sigma,
        volga: vega * d1 * d2 / sigma,
    }
}

impl Greeks {
    /// Every field zero, the base for the degenerate cases.
    const ZERO: Self = Self {
        price: 0.0,
        delta: 0.0,
        gamma: 0.0,
        vega: 0.0,
        theta: 0.0,
        rho: 0.0,
        vanna: 0.0,
        volga: 0.0,
    };
}

#[cfg(test)]
mod tests {
    // Reference values are pasted verbatim from mpmath, so keep every digit.
    #![allow(clippy::excessive_precision, clippy::unreadable_literal)]
    use super::*;
    use crate::price;
    use approx::assert_abs_diff_eq;
    use proptest::prelude::*;

    const CALL: OptionType = OptionType::Call;
    const PUT: OptionType = OptionType::Put;

    fn inputs(s: f64, k: f64, t: f64, r: f64, q: f64, v: f64) -> BsInputs {
        BsInputs::new(s, k, t, r, q, v).unwrap()
    }

    fn assert_zero(values: &[f64]) {
        for &x in values {
            assert_abs_diff_eq!(x, 0.0, epsilon = 0.0);
        }
    }

    fn assert_all_zero(g: &Greeks) {
        assert_zero(&[
            g.price, g.delta, g.gamma, g.vega, g.theta, g.rho, g.vanna, g.volga,
        ]);
    }

    /// Absolute or relative error ≤ 1e-10, whichever is looser.
    fn close(actual: f64, expected: f64, what: &str) {
        let tol = 1e-10_f64.max(1e-10 * expected.abs());
        assert!(
            (actual - expected).abs() <= tol,
            "{what}: got {actual}, expected {expected}"
        );
    }

    /// `[price, delta, gamma, vega, theta, rho, vanna, volga]`.
    fn check(option_type: OptionType, i: &BsInputs, expected: [f64; 8]) {
        let g = greeks(option_type, i);
        let names = [
            "price", "delta", "gamma", "vega", "theta", "rho", "vanna", "volga",
        ];
        let actual = [
            g.price, g.delta, g.gamma, g.vega, g.theta, g.rho, g.vanna, g.volga,
        ];
        for ((name, a), e) in names.iter().zip(actual).zip(expected) {
            close(a, e, &format!("{option_type:?} {name}"));
        }
    }

    #[test]
    fn hull_example() {
        let i = inputs(42.0, 40.0, 0.5, 0.10, 0.0, 0.20);
        check(
            CALL,
            &i,
            [
                4.7594223928715334,
                0.77913129094266894,
                0.049962670405911853,
                8.8134150596028514,
                -4.5590921945926267,
                13.982045913360281,
                -0.93160067861366839,
                21.283288061014401,
            ],
        );
        check(
            PUT,
            &i,
            [
                0.80859937290009365,
                -0.22086870905733106,
                0.049962670405911853,
                8.8134150596028514,
                -0.7541744965897705,
                -5.0425425766539992,
                -0.93160067861366839,
                21.283288061014401,
            ],
        );
    }

    #[test]
    fn dividend_case() {
        let i = inputs(100.0, 95.0, 0.75, 0.05, 0.02, 0.25);
        check(
            CALL,
            &i,
            [
                12.163047711528401,
                0.66329218416837145,
                0.016410824240452259,
                30.770295450847986,
                -6.5101067420700254,
                40.624628028981558,
                -0.33055203647297766,
                12.855934837076078,
            ],
        );
        check(
            PUT,
            &i,
            [
                5.1553234347002025,
                -0.32181975543469121,
                0.016410824240452259,
                30.770295450847986,
                -3.9051571371022472,
                -28.002974233626992,
                -0.33055203647297766,
                12.855934837076078,
            ],
        );
    }

    #[test]
    fn haug_example() {
        let i = inputs(75.0, 70.0, 0.5, 0.10, 0.05, 0.35);
        let call = greeks(CALL, &i);
        close(call.delta, 0.67560199446674141, "call delta");
        close(call.gamma, 0.018466383826289842, "call gamma");
        close(call.vega, 18.177846579004062, "call vega");
        close(call.theta, -7.8308400303306694, "call theta");
        let put = greeks(PUT, &i);
        close(put.delta, -0.29970791756159126, "put delta");
        close(put.gamma, 0.018466383826289842, "put gamma");
        close(put.vega, 18.177846579004062, "put vega");
        close(put.theta, -4.8296462289319187, "put theta");
    }

    #[test]
    fn zero_time_delta_and_zero_higher_greeks() {
        for (s, k, call_delta) in [(110.0, 100.0, 1.0), (90.0, 100.0, 0.0), (100.0, 100.0, 0.5)] {
            let i = inputs(s, k, 0.0, 0.05, 0.02, 0.2);
            let c = greeks(CALL, &i);
            let p = greeks(PUT, &i);
            assert_abs_diff_eq!(c.delta, call_delta, epsilon = 1e-15);
            assert_abs_diff_eq!(p.delta, call_delta - 1.0, epsilon = 1e-15);
            assert_abs_diff_eq!(c.price, (s - k).max(0.0), epsilon = 1e-15);
            assert_abs_diff_eq!(p.price, (k - s).max(0.0), epsilon = 1e-15);
            for g in [c, p] {
                assert_zero(&[g.gamma, g.vega, g.theta, g.rho, g.vanna, g.volga]);
            }
        }
    }

    #[test]
    fn tiny_total_vol_uses_derivatives_of_forward_intrinsic() {
        let (s, k, t, r, q) = (100.0, 95.0, 1.0, 0.05, 0.02);
        let i = inputs(s, k, t, r, q, 1e-9);
        let dq = (-q * t).exp();
        let dr = (-r * t).exp();

        // The call is in the money on the forward.
        let c = greeks(CALL, &i);
        assert_abs_diff_eq!(c.price, s * dq - k * dr, epsilon = 1e-12);
        assert_abs_diff_eq!(c.delta, dq, epsilon = 1e-12);
        assert_abs_diff_eq!(c.theta, q * s * dq - r * k * dr, epsilon = 1e-12);
        assert_abs_diff_eq!(c.rho, k * t * dr, epsilon = 1e-12);
        assert_zero(&[c.gamma, c.vega, c.vanna, c.volga]);

        // The put is out of the money on the forward, so everything is zero.
        assert_all_zero(&greeks(PUT, &i));

        // Flip the moneyness to check the put's in-the-money branch.
        let i = inputs(90.0, 100.0, t, r, q, 1e-9);
        let p = greeks(PUT, &i);
        assert_abs_diff_eq!(p.price, 100.0 * dr - 90.0 * dq, epsilon = 1e-12);
        assert_abs_diff_eq!(p.delta, -dq, epsilon = 1e-12);
        assert_abs_diff_eq!(p.theta, r * 100.0 * dr - q * 90.0 * dq, epsilon = 1e-12);
        assert_abs_diff_eq!(p.rho, -100.0 * t * dr, epsilon = 1e-12);
        assert_all_zero(&greeks(CALL, &i));
    }

    #[test]
    fn degenerate_branch_matches_closed_form_just_above_threshold() {
        // Either side of MIN_TOTAL_VOL the Greeks must agree, in and out of the money.
        let above = |s, k, v| inputs(s, k, 1.0, 0.05, 0.02, v);
        for (ty, s, k) in [(CALL, 100.0, 95.0), (PUT, 90.0, 100.0), (PUT, 100.0, 95.0)] {
            let a = greeks(ty, &above(s, k, 2e-12));
            let b = greeks(ty, &above(s, k, 5e-13));
            for (x, y) in [
                (a.price, b.price),
                (a.delta, b.delta),
                (a.gamma, b.gamma),
                (a.vega, b.vega),
                (a.theta, b.theta),
                (a.rho, b.rho),
                (a.vanna, b.vanna),
            ] {
                assert_abs_diff_eq!(x, y, epsilon = 1e-9);
            }
        }
    }

    #[test]
    fn edge_cases_are_finite() {
        let minute = 1.0 / (365.0 * 24.0 * 60.0);
        let cases = [
            inputs(100.0, 100.0, 0.0, 0.05, 0.02, 0.2),
            inputs(100.0, 100.0, minute, 0.05, 0.02, 0.2),
            inputs(100.0, 100.0, 1.0, 0.05, 0.02, 1e-9),
            inputs(100.0, 100.0, minute, 0.05, 0.02, 1e-9),
            inputs(100.0, 100.0, 1.0, 0.05, 0.02, 5.0),
            inputs(1.0, 100.0, 0.5, 0.05, 0.02, 0.25),
            inputs(100.0, 1.0, 0.5, 0.05, 0.02, 0.25),
            inputs(1.0, 100.0, minute, 0.05, 0.02, 0.25),
            inputs(100.0, 1.0, minute, 0.05, 0.02, 0.25),
            inputs(1.0, 100.0, 1.0, 0.05, 0.02, 1e-9),
            inputs(100.0, 1.0, 1.0, 0.05, 0.02, 1e-9),
            inputs(100.0, 1.0, 1.0, 0.05, 0.02, 5.0),
        ];
        for i in cases {
            for ty in [CALL, PUT] {
                let g = greeks(ty, &i);
                let all = [
                    g.price, g.delta, g.gamma, g.vega, g.theta, g.rho, g.vanna, g.volga,
                ];
                assert!(all.iter().all(|x| x.is_finite()), "{ty:?} {i:?} gave {g:?}");
            }
        }
    }

    #[test]
    fn at_the_money_gamma_peaks_and_grows_as_expiry_nears() {
        let gamma = |s: f64, t: f64| greeks(CALL, &inputs(s, 100.0, t, 0.03, 0.0, 0.2)).gamma;
        // Highest near the money, lower on either wing.
        assert!(gamma(100.0, 0.1) > gamma(90.0, 0.1));
        assert!(gamma(100.0, 0.1) > gamma(110.0, 0.1));
        // At the money it rises as expiry approaches, like 1/√T.
        assert!(gamma(100.0, 0.01) > gamma(100.0, 0.1));
        assert!(gamma(100.0, 0.001) > gamma(100.0, 0.01));
        // Away from the money it collapses instead.
        assert!(gamma(120.0, 0.001) < gamma(120.0, 0.1));
    }

    // Finite-difference checks.
    //
    // A central difference has truncation error ∝ h², which falls as h shrinks,
    // and round-off error ∝ ε·|V|/hⁿ (n = derivative order), which grows as h
    // shrinks, because V(x+h) and V(x−h) agree in most of their digits and the
    // subtraction keeps only what differs. The best h balances the two. The
    // sizes below sit near that balance for each Greek:
    //
    // - Spot bumps scale with S·σ·√T, the standard deviation of the spot move
    //   over the option's life, so a short-dated low-vol option is not stepped
    //   across its whole payoff kink by a fixed 0.1 % bump. Gamma is a second
    //   difference, so it divides by h² and its round-off is worst. It gets a
    //   bump ten times larger than delta's.
    // - Vega and rho are smooth and first order, so 1e-5 and 1e-6 are safe.
    // - Theta bumps time by 1e-6 years (about 30 seconds).
    // - Volga is a second difference in σ, so it gets 1e-4, not the 1e-5 that
    //   vega uses, and it needs a Richardson step as explained where it is used.
    //
    // The tolerance is 1e-5 relative with a floor of 1e-8, plus the round-off
    // the difference itself cannot avoid, 8·ε·(S + K)/hⁿ, since a deep
    // in-the-money price is made of terms of size S + K and no step size can
    // beat that. Where a Greek is almost zero, that noise is all that is left.

    #[derive(Clone, Copy, Debug)]
    struct Point {
        s: f64,
        k: f64,
        t: f64,
        r: f64,
        q: f64,
        v: f64,
    }

    impl Point {
        fn value(self, ty: OptionType) -> f64 {
            price(ty, &inputs(self.s, self.k, self.t, self.r, self.q, self.v))
        }
        fn analytic(self, ty: OptionType) -> Greeks {
            greeks(ty, &inputs(self.s, self.k, self.t, self.r, self.q, self.v))
        }
    }

    fn assert_fd(what: &str, p: Point, analytic: f64, numeric: f64, roundoff: f64) {
        let tol = (1e-5 * analytic.abs()).max(1e-8) + roundoff;
        assert!(
            (analytic - numeric).abs() <= tol,
            "{what} at {p:?}: analytic {analytic}, numeric {numeric}, tol {tol}"
        );
    }

    fn noise(p: Point, order: i32, h: f64) -> f64 {
        8.0 * f64::EPSILON * (p.s + p.k) / h.powi(order)
    }

    fn check_finite_differences(p: Point, ty: OptionType) {
        let g = p.analytic(ty);
        let sd = p.s * p.v * p.t.sqrt();

        let h = 1e-4 * sd;
        let up = Point { s: p.s + h, ..p };
        let dn = Point { s: p.s - h, ..p };
        let delta = (up.value(ty) - dn.value(ty)) / (2.0 * h);
        assert_fd("delta", p, g.delta, delta, noise(p, 1, h));

        let h = 1e-3 * sd;
        let up = Point { s: p.s + h, ..p };
        let dn = Point { s: p.s - h, ..p };
        let gamma = (up.value(ty) - 2.0 * p.value(ty) + dn.value(ty)) / (h * h);
        assert_fd("gamma", p, g.gamma, gamma, noise(p, 2, h));

        let h = 1e-5;
        let up = Point { v: p.v + h, ..p };
        let dn = Point { v: p.v - h, ..p };
        let vega = (up.value(ty) - dn.value(ty)) / (2.0 * h);
        assert_fd("vega", p, g.vega, vega, noise(p, 1, h));

        // Theta is −∂V/∂T, and T is the only place time enters.
        let h = 1e-6;
        let up = Point { t: p.t + h, ..p };
        let dn = Point { t: p.t - h, ..p };
        let theta = -(up.value(ty) - dn.value(ty)) / (2.0 * h);
        assert_fd("theta", p, g.theta, theta, noise(p, 1, h));

        let h = 1e-6;
        let up = Point { r: p.r + h, ..p };
        let dn = Point { r: p.r - h, ..p };
        let rho = (up.value(ty) - dn.value(ty)) / (2.0 * h);
        assert_fd("rho", p, g.rho, rho, noise(p, 1, h));

        // Volga: second difference in σ with h = 1e-4, then one Richardson step.
        // At low vol and high |d1|, d1 moves by about d1·h/σ per bump, and the
        // h² truncation term grows like (d1·h/σ)², which alone exceeds 1e-5
        // for a 5 % vol option six deviations out of the money. Combining the
        // h and h/2 differences as (4·D(h/2) − D(h)) / 3 cancels that leading
        // term and leaves an O(h⁴) error.
        let h = 1e-4;
        let second = |h: f64| {
            let up = Point { v: p.v + h, ..p };
            let dn = Point { v: p.v - h, ..p };
            (up.value(ty) - 2.0 * p.value(ty) + dn.value(ty)) / (h * h)
        };
        let volga = (4.0 * second(h / 2.0) - second(h)) / 3.0;
        assert_fd("volga", p, g.volga, volga, 2.0 * noise(p, 2, h / 2.0));

        // Vanna is a cross difference: spot bumped as for delta, σ bumped as for volga.
        let hs = 1e-4 * sd;
        let hv = 1e-4;
        let at = |ds: f64, dv: f64| {
            Point {
                s: p.s + ds,
                v: p.v + dv,
                ..p
            }
            .value(ty)
        };
        let vanna = (at(hs, hv) - at(hs, -hv) - at(-hs, hv) + at(-hs, -hv)) / (4.0 * hs * hv);
        assert_fd("vanna", p, g.vanna, vanna, noise(p, 1, hs) / hv);
    }

    prop_compose! {
        fn domain()(
            s in 10.0..1000.0f64,
            k_factor in 0.5..2.0f64,
            t in (7.0 / 365.0)..3.0f64,
            r in 0.0..0.1f64,
            q in 0.0..0.05f64,
            v in 0.05..1.5f64,
        ) -> Point {
            Point { s, k: s * k_factor, t, r, q, v }
        }
    }

    proptest! {
        #[test]
        fn matches_finite_differences_for_calls(p in domain()) {
            check_finite_differences(p, CALL);
        }

        #[test]
        fn matches_finite_differences_for_puts(p in domain()) {
            check_finite_differences(p, PUT);
        }

        #[test]
        fn call_put_identities(p in domain()) {
            let c = p.analytic(CALL);
            let u = p.analytic(PUT);
            let (s, k, t, r, q) = (p.s, p.k, p.t, p.r, p.q);
            assert_abs_diff_eq!(c.gamma, u.gamma, epsilon = 1e-14);
            assert_abs_diff_eq!(c.vega, u.vega, epsilon = 1e-12);
            assert_abs_diff_eq!(c.vanna, u.vanna, epsilon = 1e-13);
            assert_abs_diff_eq!(c.volga, u.volga, epsilon = 1e-11);
            assert_abs_diff_eq!(c.delta - u.delta, (-q * t).exp(), epsilon = 1e-12);
            assert_abs_diff_eq!(c.rho - u.rho, k * t * (-r * t).exp(), epsilon = 1e-9);
            assert_abs_diff_eq!(
                c.theta - u.theta,
                q * s * (-q * t).exp() - r * k * (-r * t).exp(),
                epsilon = 1e-10
            );
        }

        #[test]
        fn signs_and_ranges(p in domain()) {
            let c = p.analytic(CALL);
            let u = p.analytic(PUT);
            let dq = (-p.q * p.t).exp();
            let slack = 1e-12;
            prop_assert!(c.gamma >= 0.0 && u.gamma >= 0.0);
            prop_assert!(c.vega >= 0.0 && u.vega >= 0.0);
            prop_assert!(c.delta >= 0.0 && c.delta <= dq + slack, "call delta {}", c.delta);
            prop_assert!(u.delta <= 0.0 && u.delta >= -dq - slack, "put delta {}", u.delta);
        }

        #[test]
        fn price_agrees_exactly_with_greeks_price(
            p in domain(),
            t_zero in prop::bool::weighted(0.05),
        ) {
            let t = if t_zero { 0.0 } else { p.t };
            let i = inputs(p.s, p.k, t, p.r, p.q, p.v);
            for ty in [CALL, PUT] {
                // Epsilon 0 demands bit-for-bit equality. Both go through the same helper.
                assert_abs_diff_eq!(price(ty, &i), greeks(ty, &i).price, epsilon = 0.0);
            }
        }
    }
}
