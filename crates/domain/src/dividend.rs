//! Discrete cash dividends and the escrowed dividend adjustment to spot.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::sim_clock::session_open;
use crate::time::year_fraction;

/// One scheduled cash dividend: `amount` per share, going ex at the open of `ex_date`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CashDividend {
    /// The first session that trades without the dividend.
    pub ex_date: NaiveDate,
    /// Cash per share, in dollars.
    pub amount: f64,
}

/// The instant a dividend goes ex: the 09:30 New York open of its ex-date.
///
/// The simulator drops spot by the dividend at this open, so this is the moment
/// the escrowed model stops counting it.
pub fn ex_instant(ex_date: NaiveDate) -> DateTime<Utc> {
    session_open(ex_date)
}

/// The result of taking the present value of pending dividends out of spot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Escrow {
    /// The adjusted spot S* = S − Σ PV(Dᵢ).
    pub spot_star: f64,
    /// Σ PV(Dᵢ), the present value of the dividends before expiry.
    pub pv: f64,
    /// Σ τᵢ · PV(Dᵢ), where τᵢ is the time to each ex-date. It is the rate sensitivity
    /// of S*: ∂S*/∂r = +Σ τᵢ · PV(Dᵢ).
    pub tau_pv: f64,
}

/// The escrowed dividend adjustment: S* = S − Σ PV(Dᵢ) over dividends with an ex-date
/// after `now` and on or before `expiry`, discounted at the rate `rate`.
///
/// Formula: PV(Dᵢ) = Dᵢ · e^(−r τᵢ) with τᵢ the ACT/365 time from `now` to the ex-date open.
/// An option on a discrete-dividend stock is then priced with spot S* and a dividend
/// yield of zero.
///
/// # Errors
///
/// [`DomainError::EscrowedSpotNonPositive`] when S* is not above zero.
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use domain::{CashDividend, escrow};
///
/// let now = Utc.with_ymd_and_hms(2026, 10, 1, 14, 0, 0).unwrap();
/// let expiry = Utc.with_ymd_and_hms(2026, 12, 18, 21, 0, 0).unwrap();
/// let dividends = [CashDividend { ex_date: NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(), amount: 2.0 }];
/// let e = escrow(100.0, &dividends, 0.04, now, expiry)?;
/// assert!(e.spot_star > 98.0 && e.spot_star < 98.1);
/// # Ok::<(), domain::DomainError>(())
/// ```
pub fn escrow(
    spot: f64,
    dividends: &[CashDividend],
    rate: f64,
    now: DateTime<Utc>,
    expiry: DateTime<Utc>,
) -> Result<Escrow, DomainError> {
    let mut pv = 0.0;
    let mut tau_pv = 0.0;
    for dividend in dividends {
        let instant = ex_instant(dividend.ex_date);
        if instant > now && instant <= expiry {
            let tau = year_fraction(now, instant);
            let present = dividend.amount * (-rate * tau).exp();
            pv += present;
            tau_pv += tau * present;
        }
    }
    let spot_star = spot - pv;
    if spot_star.is_finite() && spot_star > 0.0 {
        Ok(Escrow {
            spot_star,
            pv,
            tau_pv,
        })
    } else {
        Err(DomainError::EscrowedSpotNonPositive { spot, pv })
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 14, 0, 0).unwrap()
    }

    fn expiry() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, 18, 21, 0, 0).unwrap()
    }

    fn div(month: u32, day: u32, amount: f64) -> CashDividend {
        CashDividend {
            ex_date: NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
            amount,
        }
    }

    #[test]
    fn present_value_is_discounted_at_the_rate() {
        let d = div(11, 5, 2.0);
        let e = escrow(100.0, &[d], 0.05, now(), expiry()).unwrap();
        let tau = year_fraction(now(), ex_instant(d.ex_date));
        assert_relative_eq!(e.pv, 2.0 * (-0.05 * tau).exp(), max_relative = 1e-14);
        assert_relative_eq!(e.tau_pv, tau * e.pv, max_relative = 1e-14);
        assert_relative_eq!(e.spot_star, 100.0 - e.pv, max_relative = 1e-14);
    }

    #[test]
    fn only_dividends_between_now_and_expiry_count() {
        let past = div(9, 20, 5.0);
        let inside = div(11, 5, 1.0);
        let after_expiry = div(12, 30, 9.0);
        let e = escrow(100.0, &[past, inside, after_expiry], 0.0, now(), expiry()).unwrap();
        assert_relative_eq!(e.pv, 1.0);
        // A dividend exactly at the present moment has already gone ex.
        let at_now = CashDividend {
            ex_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            amount: 1.0,
        };
        let open = ex_instant(at_now.ex_date);
        let e = escrow(100.0, &[at_now], 0.0, open, expiry()).unwrap();
        assert_relative_eq!(e.pv, 0.0);
    }

    #[test]
    fn a_non_positive_escrowed_spot_is_a_typed_error() {
        let result = escrow(1.0, &[div(11, 5, 2.0)], 0.0, now(), expiry());
        assert!(matches!(
            result,
            Err(DomainError::EscrowedSpotNonPositive { .. })
        ));
        assert!(escrow(1.0, &[div(11, 5, 1.0)], 0.0, now(), expiry()).is_err());
    }
}
