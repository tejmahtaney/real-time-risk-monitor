//! Time conventions: the market time zone, ACT/365 year fractions and market dates.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

/// The market time zone. Equity options expire at 16:00 in this zone.
pub const MARKET_TZ: Tz = chrono_tz::America::New_York;

/// Seconds in a 365-day year, the ACT/365 denominator.
const SECONDS_PER_YEAR: f64 = 365.0 * 86_400.0;

/// Time from `from` to `to` in years under ACT/365.
///
/// Formula: (seconds from `from` to `to`) / (365 × 86,400). Returns 0 when
/// `to` is not after `from`, so a time to expiry never goes negative.
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use domain::year_fraction;
///
/// let a = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
/// let b = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
/// assert_eq!(year_fraction(a, b), 1.0);
/// assert_eq!(year_fraction(b, a), 0.0);
/// ```
pub fn year_fraction(from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
    let millis = to.signed_duration_since(from).num_milliseconds();
    if millis <= 0 {
        return 0.0;
    }
    millis as f64 / 1_000.0 / SECONDS_PER_YEAR
}

/// The calendar date in the market time zone (`America/New_York`) at `instant`.
///
/// Just after midnight UTC it is still the previous day in New York, which is
/// the date that matters for "today's expiry".
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use domain::market_date;
///
/// let t = Utc.with_ymd_and_hms(2026, 7, 2, 1, 0, 0).unwrap();
/// assert_eq!(market_date(t), NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
/// ```
pub fn market_date(instant: DateTime<Utc>) -> NaiveDate {
    instant.with_timezone(&MARKET_TZ).date_naive()
}

#[cfg(test)]
mod tests {
    use approx::{assert_abs_diff_eq, assert_relative_eq};
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn one_day_is_one_365th() {
        assert_relative_eq!(
            year_fraction(utc(2026, 3, 1, 0), utc(2026, 3, 2, 0)),
            1.0 / 365.0,
            max_relative = 1e-15
        );
    }

    #[test]
    fn a_365_day_span_is_one_year() {
        assert_relative_eq!(
            year_fraction(utc(2026, 1, 1, 0), utc(2027, 1, 1, 0)),
            1.0,
            max_relative = 1e-15
        );
    }

    #[test]
    fn leap_year_is_a_little_over_one() {
        assert_relative_eq!(
            year_fraction(utc(2028, 1, 1, 0), utc(2029, 1, 1, 0)),
            366.0 / 365.0,
            max_relative = 1e-15
        );
    }

    #[test]
    fn zero_when_not_after() {
        let t = utc(2026, 5, 5, 12);
        assert_abs_diff_eq!(year_fraction(t, t), 0.0);
        assert_abs_diff_eq!(year_fraction(utc(2026, 5, 6, 0), t), 0.0);
    }

    #[test]
    fn market_date_follows_new_york() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        // 01:00 UTC on 2 July is 21:00 on 1 July in New York (EDT).
        assert_eq!(market_date(utc(2026, 7, 2, 1)), d(2026, 7, 1));
        // 12:00 UTC is the same day.
        assert_eq!(market_date(utc(2026, 7, 2, 12)), d(2026, 7, 2));
        // Winter, 04:59 UTC is 23:59 the day before in EST.
        let t = Utc.with_ymd_and_hms(2026, 1, 2, 4, 59, 0).unwrap();
        assert_eq!(market_date(t), d(2026, 1, 1));
    }
}
