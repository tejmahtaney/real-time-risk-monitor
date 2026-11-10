//! Implied volatility: the σ at which the Black-Scholes-Merton price equals a market price.

use crate::bsm::Terms;
use crate::error::PricingError;
use crate::greeks::greeks;
use crate::inputs::{BsInputs, OptionType};

/// Lower end of the starting bracket. A root below this is reported as this value.
const VOL_FLOOR: f64 = 1e-6;
/// Upper end of the starting bracket, 1000 % volatility.
const VOL_CEILING: f64 = 10.0;
/// The starting guess is clamped to this range.
const GUESS_RANGE: (f64, f64) = (0.05, 3.0);
/// Vega below this is too flat to trust a Newton step.
const MIN_VEGA: f64 = 1e-12;
/// Stop when the price error is within this factor of max(1, price).
const PRICE_TOLERANCE: f64 = 1e-12;
/// Stop when the bracket is narrower than this.
const BRACKET_TOLERANCE: f64 = 1e-14;
/// The solver gives up after this many evaluations.
const MAX_ITERATIONS: u32 = 100;
/// Slack at the intrinsic and upper bounds, as a factor of max(S, K).
const BOUND_TOLERANCE: f64 = 1e-12;

/// A market price to turn into an implied volatility.
///
/// Every field is public. The request is validated by [`implied_volatility`],
/// with the same rules as [`BsInputs::new`] except that there is no volatility.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IvRequest {
    /// Call or put.
    pub option_type: OptionType,
    /// The observed option price per unit of underlying. Must be finite.
    pub price: f64,
    /// Spot price of the underlying, positive.
    pub spot: f64,
    /// Strike price, positive.
    pub strike: f64,
    /// Time to expiry in years, zero or positive.
    pub time_to_expiry: f64,
    /// Continuously compounded risk-free rate as a decimal, within -1.0 to 1.0.
    pub rate: f64,
    /// Continuous dividend yield as a decimal, within -1.0 to 1.0.
    pub dividend_yield: f64,
}

/// A solved implied volatility.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IvSolution {
    /// The implied volatility as a decimal, 0.2 meaning 20 %.
    pub volatility: f64,
    /// How many times the price and vega were evaluated.
    pub iterations: u32,
}

/// Why an implied volatility could not be found.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum IvError {
    /// Time to expiry is zero, so the price is intrinsic value whatever σ is.
    #[error("the option has expired, so implied volatility is undefined")]
    Expired,
    /// The price is under the discounted forward intrinsic value, so no σ can produce it.
    #[error("price {price} is below the intrinsic value {intrinsic}")]
    BelowIntrinsic {
        /// The rejected price.
        price: f64,
        /// max(ω·(S·e^(−qT) − K·e^(−rT)), 0), the lowest price any σ gives.
        intrinsic: f64,
    },
    /// The price is at or above the no-arbitrage upper bound, which only σ = ∞ reaches.
    #[error("price {price} is at or above the upper bound {upper_bound}")]
    AboveUpperBound {
        /// The rejected price.
        price: f64,
        /// S·e^(−qT) for a call, K·e^(−rT) for a put.
        upper_bound: f64,
    },
    /// The solver ran out of iterations, or the root lies above the 1000 % vol ceiling.
    #[error("no convergence after {iterations} iterations, last estimate {last_estimate}")]
    NoConvergence {
        /// Evaluations made.
        iterations: u32,
        /// The volatility at the last evaluation.
        last_estimate: f64,
    },
    /// An input broke a validation rule.
    #[error("invalid input: {0}")]
    InvalidInput(#[from] PricingError),
}

/// Implied volatility of a European option: the σ for which the
/// Black-Scholes-Merton price equals `req.price`.
///
/// The price is strictly increasing in σ, with vega = S·e^(−qT)·φ(d1)·√T > 0,
/// so at most one root exists. It exists exactly when the price lies between
/// the discounted forward intrinsic value, max(ω·(S·e^(−qT) − K·e^(−rT)), 0),
/// and the upper bound, S·e^(−qT) for a call or K·e^(−rT) for a put, with
/// ω = +1 for a call and −1 for a put. A price within 1e-12 × max(S, K) of
/// either bound counts as on it.
///
/// # Method
///
/// Safeguarded Newton. The start is the Brenner-Subrahmanyam guess,
/// σ₀ = √(2π/T)·price/S, clamped to 0.05 to 3.0. The solver keeps a bracket
/// [lo, hi], initially [1e-6, 10], and every evaluation of BS(σ) tightens it,
/// because BS is increasing: a value above the target puts the root below σ.
/// Each step is Newton's, σ − (BS(σ) − price) / vega(σ), unless vega is under
/// 1e-12 or the step lands outside the bracket, when it is the bracket's
/// midpoint instead. That keeps the fast quadratic convergence where the curve
/// is well behaved and the guaranteed halving where it is flat. One call to
/// [`greeks`] gives both BS(σ) and vega.
///
/// It stops when |BS(σ) − price| ≤ 1e-12 × max(1, price) or the bracket is
/// narrower than 1e-14, and gives up after 100 evaluations.
///
/// # Limits
///
/// - Where vega is tiny (deep in or out of the money, short-dated) many σ give
///   the same price to within the tolerance, so the returned σ reproduces the
///   price but may differ from the σ that made it.
/// - A root below 1e-6, for example a price exactly on intrinsic, is returned
///   as 1e-6. A root above 10 is [`IvError::NoConvergence`].
///
/// Jäckel's "Let's Be Rational" (2015) is the production-grade alternative.
/// It reaches machine precision in two steps from a rational-function start,
/// with no bracket, but it is out of scope here.
///
/// # Errors
///
/// [`IvError::InvalidInput`] for a failed validation rule or a non-finite price,
/// [`IvError::Expired`] when T = 0, [`IvError::BelowIntrinsic`] and
/// [`IvError::AboveUpperBound`] when no σ can give the price, and
/// [`IvError::NoConvergence`] when the search fails.
///
/// ```
/// use pricing::{implied_volatility, IvRequest, OptionType};
///
/// // Hull's example: this call price comes from a volatility of 20 %.
/// let req = IvRequest {
///     option_type: OptionType::Call,
///     price: 4.7594223928715334,
///     spot: 42.0,
///     strike: 40.0,
///     time_to_expiry: 0.5,
///     rate: 0.10,
///     dividend_yield: 0.0,
/// };
/// let solution = implied_volatility(&req)?;
/// assert!((solution.volatility - 0.20).abs() < 1e-12);
/// # Ok::<(), pricing::IvError>(())
/// ```
pub fn implied_volatility(req: &IvRequest) -> Result<IvSolution, IvError> {
    if !req.price.is_finite() {
        return Err(PricingError::NonFinite {
            field: "price",
            value: req.price,
        }
        .into());
    }
    // A placeholder σ of 1 lets `BsInputs` run the shared validation. Its
    // volatility rule cannot fail.
    let base = BsInputs::new(
        req.spot,
        req.strike,
        req.time_to_expiry,
        req.rate,
        req.dividend_yield,
        1.0,
    )?;
    if req.time_to_expiry == 0.0 {
        return Err(IvError::Expired);
    }

    let terms = Terms::new(&base);
    let bound_tolerance = BOUND_TOLERANCE * req.spot.max(req.strike);
    let intrinsic = terms.forward_intrinsic(req.option_type);
    if req.price < intrinsic - bound_tolerance {
        return Err(IvError::BelowIntrinsic {
            price: req.price,
            intrinsic,
        });
    }
    let upper_bound = match req.option_type {
        OptionType::Call => terms.forward_spot,
        OptionType::Put => terms.discounted_strike,
    };
    if req.price >= upper_bound - bound_tolerance {
        return Err(IvError::AboveUpperBound {
            price: req.price,
            upper_bound,
        });
    }

    let price_tolerance = PRICE_TOLERANCE * req.price.max(1.0);
    // Brenner-Subrahmanyam: at the money, price ≈ S·σ·√(T/2π). It is exact only
    // there, so it serves as a start point and nothing more.
    let guess = (2.0 * std::f64::consts::PI / req.time_to_expiry).sqrt() * req.price / req.spot;
    let mut sigma = guess.clamp(GUESS_RANGE.0, GUESS_RANGE.1);
    let (mut lo, mut hi) = (VOL_FLOOR, VOL_CEILING);

    for iteration in 1..=MAX_ITERATIONS {
        let g = greeks(req.option_type, &base.with_volatility(sigma)?);
        let error = g.price - req.price;
        if error.abs() <= price_tolerance {
            return Ok(IvSolution {
                volatility: sigma,
                iterations: iteration,
            });
        }
        if error > 0.0 {
            hi = sigma;
        } else {
            lo = sigma;
        }
        if hi - lo < BRACKET_TOLERANCE {
            // The bracket collapsing onto the floor is a root below the floor.
            // Collapsing onto the ceiling with the price still out of reach is
            // a root above it.
            if error < 0.0 && error.abs() > bound_tolerance {
                return Err(IvError::NoConvergence {
                    iterations: iteration,
                    last_estimate: sigma,
                });
            }
            return Ok(IvSolution {
                volatility: sigma,
                iterations: iteration,
            });
        }
        let newton = sigma - error / g.vega;
        sigma = if g.vega >= MIN_VEGA && newton > lo && newton < hi {
            newton
        } else {
            lo.midpoint(hi)
        };
    }
    Err(IvError::NoConvergence {
        iterations: MAX_ITERATIONS,
        last_estimate: sigma,
    })
}

#[cfg(test)]
mod tests {
    // The Hull reference price is pasted verbatim from mpmath, so keep every digit.
    #![allow(clippy::excessive_precision, clippy::unreadable_literal)]
    use super::*;
    use crate::price;
    use proptest::prelude::*;

    const CALL: OptionType = OptionType::Call;
    const PUT: OptionType = OptionType::Put;

    fn request(option_type: OptionType, price: f64) -> IvRequest {
        IvRequest {
            option_type,
            price,
            spot: 100.0,
            strike: 100.0,
            time_to_expiry: 1.0,
            rate: 0.0,
            dividend_yield: 0.0,
        }
    }

    /// The request whose price is the BSM price at `vol`.
    fn priced(
        option_type: OptionType,
        s: f64,
        k: f64,
        t: f64,
        r: f64,
        q: f64,
        vol: f64,
    ) -> IvRequest {
        let inputs = BsInputs::new(s, k, t, r, q, vol).unwrap();
        IvRequest {
            option_type,
            price: price(option_type, &inputs),
            spot: s,
            strike: k,
            time_to_expiry: t,
            rate: r,
            dividend_yield: q,
        }
    }

    #[test]
    fn hull_call_recovers_twenty_percent() {
        let req = IvRequest {
            option_type: CALL,
            price: 4.7594223928715334,
            spot: 42.0,
            strike: 40.0,
            time_to_expiry: 0.5,
            rate: 0.10,
            dividend_yield: 0.0,
        };
        let solution = implied_volatility(&req).unwrap();
        assert!(
            (solution.volatility - 0.20).abs() < 1e-12,
            "got {}",
            solution.volatility
        );
    }

    #[test]
    fn hull_put_recovers_twenty_percent() {
        let req = IvRequest {
            option_type: PUT,
            price: 0.80859937290009365,
            spot: 42.0,
            strike: 40.0,
            time_to_expiry: 0.5,
            rate: 0.10,
            dividend_yield: 0.0,
        };
        let solution = implied_volatility(&req).unwrap();
        assert!((solution.volatility - 0.20).abs() < 1e-12);
    }

    #[test]
    fn expired_option_is_an_error() {
        let mut req = request(CALL, 5.0);
        req.time_to_expiry = 0.0;
        assert_eq!(implied_volatility(&req), Err(IvError::Expired));
    }

    #[test]
    fn price_below_intrinsic_is_an_error() {
        // Forward intrinsic of the call is 100 − 90·e^(−0.05) ≈ 14.4, so 10 is too low.
        let mut req = request(CALL, 10.0);
        req.strike = 90.0;
        req.rate = 0.05;
        let expected = 100.0 - 90.0 * (-0.05_f64).exp();
        assert!(matches!(
            implied_volatility(&req),
            Err(IvError::BelowIntrinsic { price, intrinsic })
                if (price - 10.0).abs() < 1e-15 && (intrinsic - expected).abs() < 1e-12
        ));
        // A negative price is below the zero intrinsic of an out-of-the-money option.
        assert!(matches!(
            implied_volatility(&request(PUT, -0.01)),
            Err(IvError::BelowIntrinsic { .. })
        ));
    }

    #[test]
    fn price_on_intrinsic_within_tolerance_does_not_fail() {
        let mut req = request(CALL, 0.0);
        req.strike = 90.0;
        req.rate = 0.05;
        let intrinsic = 100.0 - 90.0 * (-0.05_f64).exp();
        let tolerance = 1e-12 * 100.0;
        // Exactly on it, and rounding just under it.
        req.price = intrinsic;
        assert!(implied_volatility(&req).is_ok());
        req.price = intrinsic - 0.5 * tolerance;
        assert!(implied_volatility(&req).is_ok());
        // Beyond the tolerance it fails.
        req.price = intrinsic - 2.0 * tolerance;
        assert!(matches!(
            implied_volatility(&req),
            Err(IvError::BelowIntrinsic { .. })
        ));
        // At the money on the forward the intrinsic value is 0 and the root is
        // below the floor, which is returned as the floor.
        let solution = implied_volatility(&request(PUT, 0.0)).unwrap();
        assert!(solution.volatility <= 2.0 * VOL_FLOOR);
    }

    #[test]
    fn price_at_or_above_upper_bound_is_an_error() {
        // The upper bound of a call is the spot, here 100.
        for price in [100.0, 100.5, 100.0 - 0.5e-10] {
            assert!(
                matches!(
                    implied_volatility(&request(CALL, price)),
                    Err(IvError::AboveUpperBound { upper_bound, .. })
                        if (upper_bound - 100.0).abs() < 1e-12
                ),
                "price {price} should be above the bound"
            );
        }
        // The upper bound of a put is K·e^(−rT).
        let mut req = request(PUT, 100.0);
        req.rate = 0.05;
        let expected = 100.0 * (-0.05_f64).exp();
        assert!(matches!(
            implied_volatility(&req),
            Err(IvError::AboveUpperBound { upper_bound, .. }) if (upper_bound - expected).abs() < 1e-12
        ));
        // Just clear of the tolerance the price is accepted as reachable, or fails later.
        assert!(!matches!(
            implied_volatility(&request(CALL, 100.0 - 2e-10)),
            Err(IvError::AboveUpperBound { .. })
        ));
    }

    #[test]
    fn root_above_the_ceiling_is_no_convergence() {
        // 1 − price/S = 1e-9 needs σ ≈ 12, above the 10.0 ceiling, yet the price
        // is more than the bound tolerance below the spot.
        assert!(matches!(
            implied_volatility(&request(CALL, 100.0 - 1e-7)),
            Err(IvError::NoConvergence { iterations, last_estimate })
                if iterations <= MAX_ITERATIONS && (last_estimate - VOL_CEILING).abs() < 1e-9
        ));
    }

    #[test]
    fn invalid_inputs_are_wrapped() {
        let bad = |edit: fn(&mut IvRequest)| {
            let mut req = request(CALL, 5.0);
            edit(&mut req);
            implied_volatility(&req)
        };
        assert_eq!(
            bad(|r| r.spot = -1.0),
            Err(IvError::InvalidInput(PricingError::InvalidSpot(-1.0)))
        );
        assert_eq!(
            bad(|r| r.strike = 0.0),
            Err(IvError::InvalidInput(PricingError::InvalidStrike(0.0)))
        );
        assert_eq!(
            bad(|r| r.time_to_expiry = -0.5),
            Err(IvError::InvalidInput(PricingError::InvalidTimeToExpiry(
                -0.5
            )))
        );
        assert_eq!(
            bad(|r| r.rate = 2.0),
            Err(IvError::InvalidInput(PricingError::RateOutOfRange(2.0)))
        );
        assert_eq!(
            bad(|r| r.dividend_yield = -2.0),
            Err(IvError::InvalidInput(
                PricingError::DividendYieldOutOfRange(-2.0)
            ))
        );
        assert!(matches!(
            bad(|r| r.price = f64::NAN),
            Err(IvError::InvalidInput(PricingError::NonFinite {
                field: "price",
                ..
            }))
        ));
        assert!(matches!(
            bad(|r| r.spot = f64::INFINITY),
            Err(IvError::InvalidInput(PricingError::NonFinite {
                field: "spot",
                ..
            }))
        ));
    }

    #[test]
    fn deep_out_of_the_money_short_dated_option_solves() {
        // At the start guess of 0.05 the option is worth about 1e-100 and vega is about 1e-96,
        // so plain Newton would jump to σ ≈ 6e92. The bracket rejects that step.
        let req = priced(CALL, 100.0, 140.0, 0.1, 0.02, 0.0, 0.3);
        let solution = implied_volatility(&req).unwrap();
        assert!((solution.volatility - 0.3).abs() < 1e-6, "{solution:?}");
    }

    #[test]
    fn extreme_vols_are_recovered() {
        // The strike sits on the forward, S·e^((r − q)·T), where vega is largest, so σ is identifiable even at 0.1 %.
        let forward_strike = 100.0 * (0.02_f64).exp();
        for vol in [0.001, 0.01, 4.0, 8.0] {
            let req = priced(CALL, 100.0, forward_strike, 1.0, 0.03, 0.01, vol);
            let solution = implied_volatility(&req).unwrap();
            assert!(
                (solution.volatility - vol).abs() < 1e-8 * vol.max(1.0),
                "{vol}: {solution:?}"
            );
        }
    }

    /// Iteration counts over a grid of the round-trip domain, calls and puts.
    /// Returns `(mean, max)`.
    fn grid_iteration_stats() -> (f64, u32) {
        let (mut total, mut max, mut cases) = (0_u64, 0_u32, 0_usize);
        for s in [10.0, 100.0, 1000.0] {
            for k_factor in [0.5, 0.7, 0.9, 1.0, 1.1, 1.4, 2.0] {
                for t in [7.0 / 365.0, 0.1, 0.5, 1.0, 3.0] {
                    for (r, q) in [(0.0, 0.0), (0.05, 0.02), (0.1, 0.05)] {
                        for v in [0.02, 0.05, 0.1, 0.2, 0.4, 0.8, 1.5, 2.0] {
                            for option_type in [CALL, PUT] {
                                let req = priced(option_type, s, s * k_factor, t, r, q, v);
                                let solution = implied_volatility(&req).unwrap();
                                total += u64::from(solution.iterations);
                                max = max.max(solution.iterations);
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
        (total as f64 / cases as f64, max)
    }

    #[test]
    fn grid_needs_few_iterations() {
        // Measured on this grid: mean 8.64, maximum 33.
        let (mean, max) = grid_iteration_stats();
        assert!(max <= 64, "max {max}");
        assert!(mean < 12.0, "mean {mean}");
    }

    prop_compose! {
        /// The Part 3 domain for everything but σ, which runs 0.02 to 2.0.
        fn domain()(
            s in 10.0..1000.0f64,
            k_factor in 0.5..2.0f64,
            t in (7.0 / 365.0)..3.0f64,
            r in 0.0..0.1f64,
            q in 0.0..0.05f64,
            v in 0.02..2.0f64,
            is_call in any::<bool>(),
        ) -> (OptionType, f64, f64, f64, f64, f64, f64) {
            let option_type = if is_call { CALL } else { PUT };
            (option_type, s, s * k_factor, t, r, q, v)
        }
    }

    proptest! {
        // Where vega × 1e-8 exceeds the price tolerance, a σ error of 1e-8 moves
        // the price by more than the solver accepts, so σ itself is pinned down
        // to 1e-8. Where vega is smaller, a whole range of σ gives the same price
        // to within the tolerance (deep in or out of the money, short-dated) and
        // σ is not identifiable. All that can be required is that the recovered σ
        // reproduces the price, to 1e-10 × max(S, K).
        #[test]
        fn round_trip_recovers_sigma_or_reproduces_price(
            (option_type, s, k, t, r, q, v) in domain(),
        ) {
            let req = priced(option_type, s, k, t, r, q, v);
            let solution = implied_volatility(&req).unwrap();
            let at_true = BsInputs::new(s, k, t, r, q, v).unwrap();
            let vega = greeks(option_type, &at_true).vega;
            let price_tolerance = PRICE_TOLERANCE * req.price.max(1.0);
            if vega * 1e-8 > price_tolerance {
                prop_assert!(
                    (solution.volatility - v).abs() <= 1e-8,
                    "sigma {} vs {v}, vega {vega}", solution.volatility
                );
            } else {
                let at_solution = BsInputs::new(s, k, t, r, q, solution.volatility).unwrap();
                let error = (price(option_type, &at_solution) - req.price).abs();
                prop_assert!(
                    error <= 1e-10 * s.max(k),
                    "price error {error}, sigma {} vs {v}, vega {vega}", solution.volatility
                );
            }
        }

        // Pure bisection from [1e-6, 10] to 1e-14 takes about 50 steps, so more
        // than 64 would mean the Newton steps are hurting.
        #[test]
        fn never_needs_more_than_64_iterations(
            (option_type, s, k, t, r, q, v) in domain(),
        ) {
            let solution = implied_volatility(&priced(option_type, s, k, t, r, q, v)).unwrap();
            prop_assert!(solution.iterations <= 64, "{} iterations", solution.iterations);
        }
    }
}
