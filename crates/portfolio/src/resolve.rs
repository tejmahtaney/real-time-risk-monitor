//! Resolving unresolved [`PositionSpec`]s into [`Position`]s against a market.

use chrono::{Datelike, Days, NaiveDate, Weekday};
use domain::{
    Instrument, InstrumentKind, MarketState, OptionContract, OptionType, Position, UnderlyingState,
    market_date, year_fraction,
};
use pricing::BsInputs;

use crate::error::PortfolioError;
use crate::spec::{ExpirySpec, PositionSpec, StrikeSpec};

/// Rounds a raw strike to a listed strike.
///
/// The step depends on the raw value: 1 below $100, 5 from $100 up to but not
/// including $1,000, and 10 from $1,000. The result is the nearest multiple of
/// the step, with halves rounding away from zero.
///
/// ```
/// use portfolio::round_strike;
///
/// assert_eq!(round_strike(87.4), 87.0);
/// assert_eq!(round_strike(231.2), 230.0);
/// assert_eq!(round_strike(1234.0), 1230.0);
/// ```
pub fn round_strike(raw: f64) -> f64 {
    let step = if raw < 100.0 {
        1.0
    } else if raw < 1000.0 {
        5.0
    } else {
        10.0
    };
    (raw / step).round() * step
}

/// Resolves an expiry against the start date.
///
/// Dates are taken in the market time zone. [`ExpirySpec::Today`] is the start
/// date itself. [`ExpirySpec::FirstFridayAfter`] is the first Friday on or
/// after the start date plus N days, so a date that is already a Friday is
/// kept. Returns `None` only if the date is out of the calendar's range.
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use portfolio::{resolve_expiry, ExpirySpec};
///
/// // Friday 2 October 2026, midday in New York.
/// let start = Utc.with_ymd_and_hms(2026, 10, 2, 16, 0, 0).unwrap();
/// let friday = |d| NaiveDate::from_ymd_opt(2026, 10, d);
/// assert_eq!(resolve_expiry(ExpirySpec::Today, start), friday(2));
/// assert_eq!(resolve_expiry(ExpirySpec::FirstFridayAfter(7), start), friday(9));
/// assert_eq!(resolve_expiry(ExpirySpec::FirstFridayAfter(10), start), friday(16));
/// ```
pub fn resolve_expiry(spec: ExpirySpec, start: chrono::DateTime<chrono::Utc>) -> Option<NaiveDate> {
    let today = market_date(start);
    match spec {
        ExpirySpec::Date(d) => Some(d),
        ExpirySpec::Today => Some(today),
        ExpirySpec::FirstFridayAfter(days) => {
            let target = today.checked_add_days(Days::new(u64::from(days)))?;
            let ahead = (Weekday::Fri.num_days_from_monday() + 7
                - target.weekday().num_days_from_monday())
                % 7;
            target.checked_add_days(Days::new(u64::from(ahead)))
        }
    }
}

/// Resolves unresolved specs into positions against a market.
///
/// Relative strikes are the spot times (1 + offset) rounded by [`round_strike`].
/// Relative expiries count from the New York date of `start`. A blank entry
/// price becomes the theoretical value at `start`, so day-one unrealised P&L
/// is zero: the spot for equity, and the Black-Scholes-Merton price for an
/// option, using the position's volatility override if it has one. Loading and
/// resolving are separate steps, so the same specs can be resolved against
/// different markets. Duplicate ids are caught when positions are added to a book.
///
/// # Errors
///
/// [`PortfolioError::MissingUnderlying`] when the market has no data for a
/// position's underlying, [`PortfolioError::InvalidStrike`] when a strike
/// resolves to zero or less, plus the domain and pricing errors for bad numbers.
pub fn resolve(
    specs: &[PositionSpec],
    market: &MarketState,
    start: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<Position>, PortfolioError> {
    specs
        .iter()
        .map(|spec| resolve_one(spec, market, start))
        .collect()
}

fn resolve_one(
    spec: &PositionSpec,
    market: &MarketState,
    start: chrono::DateTime<chrono::Utc>,
) -> Result<Position, PortfolioError> {
    let state =
        market
            .underlying(&spec.underlying)
            .ok_or_else(|| PortfolioError::MissingUnderlying {
                id: spec.id.clone(),
                symbol: spec.underlying.clone(),
            })?;

    let instrument = match spec.kind {
        InstrumentKind::Equity => Instrument::Equity {
            symbol: spec.underlying.clone(),
        },
        InstrumentKind::Call | InstrumentKind::Put => {
            let strike_spec = spec.strike.ok_or_else(|| PortfolioError::MissingField {
                id: spec.id.clone(),
                field: "strike",
            })?;
            let expiry_spec = spec.expiry.ok_or_else(|| PortfolioError::MissingField {
                id: spec.id.clone(),
                field: "expiry",
            })?;
            let strike = match strike_spec {
                StrikeSpec::Absolute(k) => k,
                StrikeSpec::Atm { offset_pct } => {
                    round_strike(state.spot() * (1.0 + offset_pct / 100.0))
                }
            };
            if !strike.is_finite() || strike <= 0.0 {
                return Err(PortfolioError::InvalidStrike {
                    id: spec.id.clone(),
                    strike,
                });
            }
            let expiry = resolve_expiry(expiry_spec, start).ok_or_else(|| {
                PortfolioError::InvalidExpiry {
                    id: spec.id.clone(),
                }
            })?;
            let option_type = if spec.kind == InstrumentKind::Call {
                OptionType::Call
            } else {
                OptionType::Put
            };
            Instrument::Option(OptionContract::new(
                spec.underlying.clone(),
                option_type,
                strike,
                expiry,
            )?)
        }
    };

    let entry_price = match spec.entry_price {
        Some(p) => p,
        None => theoretical_value(&instrument, state, market.rate(), spec.iv, start)?,
    };
    Ok(Position::new(
        spec.id.clone(),
        instrument,
        spec.quantity,
        entry_price,
        spec.iv,
    )?)
}

/// The per-unit theoretical value of an instrument at `at`.
fn theoretical_value(
    instrument: &Instrument,
    state: &UnderlyingState,
    rate: f64,
    iv_override: Option<f64>,
    at: chrono::DateTime<chrono::Utc>,
) -> Result<f64, PortfolioError> {
    match instrument {
        Instrument::Equity { .. } => Ok(state.spot()),
        Instrument::Option(c) => {
            let inputs = BsInputs::new(
                state.spot(),
                c.strike,
                year_fraction(at, c.expiry_datetime()),
                rate,
                state.dividend_yield(),
                iv_override.unwrap_or_else(|| state.vol()),
            )?;
            Ok(pricing::price(c.option_type, &inputs))
        }
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::{DateTime, TimeZone, Utc};
    use domain::{PositionId, Symbol};

    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn market(spot: f64) -> MarketState {
        MarketState::new(
            utc(2026, 10, 1, 14),
            0.04,
            [(
                Symbol::new("SPY").unwrap(),
                UnderlyingState::new(spot, 0.2, 0.01).unwrap(),
            )],
        )
        .unwrap()
    }

    fn spec(kind: InstrumentKind, strike: Option<&str>, expiry: Option<&str>) -> PositionSpec {
        PositionSpec {
            id: PositionId::new("p").unwrap(),
            underlying: Symbol::new("SPY").unwrap(),
            kind,
            strike: strike.map(|s| s.parse().unwrap()),
            expiry: expiry.map(|s| s.parse().unwrap()),
            quantity: 1.0,
            entry_price: None,
            iv: None,
        }
    }

    #[test]
    fn strike_rounding_bands() {
        assert_relative_eq!(round_strike(87.4), 87.0);
        assert_relative_eq!(round_strike(87.6), 88.0);
        assert_relative_eq!(round_strike(99.4), 99.0);
        assert_relative_eq!(round_strike(231.2), 230.0);
        assert_relative_eq!(round_strike(232.6), 235.0);
        assert_relative_eq!(round_strike(997.6), 1000.0);
        assert_relative_eq!(round_strike(1234.0), 1230.0);
        assert_relative_eq!(round_strike(1236.0), 1240.0);
    }

    #[test]
    fn strike_rounding_boundaries() {
        // The step is chosen from the raw value: 99.5 is still in the $1 band.
        assert_relative_eq!(round_strike(99.5), 100.0);
        assert_relative_eq!(round_strike(100.0), 100.0);
        assert_relative_eq!(round_strike(102.4), 100.0);
        assert_relative_eq!(round_strike(102.5), 105.0);
        assert_relative_eq!(round_strike(999.0), 1000.0);
        assert_relative_eq!(round_strike(1000.0), 1000.0);
        assert_relative_eq!(round_strike(1004.9), 1000.0);
        assert_relative_eq!(round_strike(1005.0), 1010.0);
    }

    #[test]
    fn relative_strikes_resolve_against_spot() {
        let market = market(600.0);
        let at = utc(2026, 10, 1, 14);
        let strike_of = |text: &str| {
            let s = spec(InstrumentKind::Call, Some(text), Some("+30d"));
            match &resolve(&[s], &market, at).unwrap()[0].instrument {
                Instrument::Option(c) => c.strike,
                Instrument::Equity { .. } => f64::NAN,
            }
        };
        assert_relative_eq!(strike_of("ATM"), 600.0);
        assert_relative_eq!(strike_of("ATM+5%"), 630.0);
        assert_relative_eq!(strike_of("ATM-5%"), 570.0);
        assert_relative_eq!(strike_of("ATM-10%"), 540.0);
        assert_relative_eq!(strike_of("612.5"), 612.5);
    }

    #[test]
    fn same_specs_resolve_differently_against_different_markets() {
        let s = [spec(InstrumentKind::Put, Some("ATM"), Some("+30d"))];
        let at = utc(2026, 10, 1, 14);
        let strike = |m: &MarketState| match &resolve(&s, m, at).unwrap()[0].instrument {
            Instrument::Option(c) => c.strike,
            Instrument::Equity { .. } => f64::NAN,
        };
        assert_relative_eq!(strike(&market(600.0)), 600.0);
        assert_relative_eq!(strike(&market(450.0)), 450.0);
    }

    #[test]
    fn expiry_today_and_absolute() {
        let start = utc(2026, 10, 1, 14);
        assert_eq!(
            resolve_expiry(ExpirySpec::Today, start),
            Some(date(2026, 10, 1))
        );
        assert_eq!(
            resolve_expiry(ExpirySpec::Date(date(2027, 1, 15)), start),
            Some(date(2027, 1, 15))
        );
    }

    #[test]
    fn expiry_today_uses_the_new_york_date() {
        // 01:00 UTC on 2 October is 21:00 on 1 October in New York.
        let start = utc(2026, 10, 2, 1);
        assert_eq!(
            resolve_expiry(ExpirySpec::Today, start),
            Some(date(2026, 10, 1))
        );
    }

    #[test]
    fn expiry_moves_to_the_next_friday() {
        // Thursday 1 October 2026.
        let start = utc(2026, 10, 1, 14);
        let plus = |n| resolve_expiry(ExpirySpec::FirstFridayAfter(n), start);
        assert_eq!(plus(1), Some(date(2026, 10, 2))); // Friday, kept
        assert_eq!(plus(2), Some(date(2026, 10, 9))); // Saturday -> next Friday
        assert_eq!(plus(3), Some(date(2026, 10, 9))); // Sunday
        assert_eq!(plus(7), Some(date(2026, 10, 9))); // Thursday -> Friday
        assert_eq!(plus(14), Some(date(2026, 10, 16)));
        assert_eq!(plus(30), Some(date(2026, 11, 6)));
    }

    #[test]
    fn expiry_when_the_start_date_is_a_friday() {
        // Friday 2 October 2026.
        let start = utc(2026, 10, 2, 14);
        // +0d is today, a Friday anyway.
        assert_eq!(
            resolve_expiry(ExpirySpec::Today, start),
            Some(date(2026, 10, 2))
        );
        // A Friday target is kept: 7 and 14 days on are Fridays.
        assert_eq!(
            resolve_expiry(ExpirySpec::FirstFridayAfter(7), start),
            Some(date(2026, 10, 9))
        );
        assert_eq!(
            resolve_expiry(ExpirySpec::FirstFridayAfter(14), start),
            Some(date(2026, 10, 16))
        );
        // One day on is a Saturday, so it moves a week.
        assert_eq!(
            resolve_expiry(ExpirySpec::FirstFridayAfter(1), start),
            Some(date(2026, 10, 9))
        );
    }

    #[test]
    fn expiry_today_is_not_moved_to_a_friday() {
        // Wednesday 30 September 2026.
        let start = utc(2026, 9, 30, 14);
        assert_eq!(
            resolve_expiry(ExpirySpec::Today, start),
            Some(date(2026, 9, 30))
        );
    }

    #[test]
    fn blank_entry_price_is_the_theoretical_value() {
        let m = market(600.0);
        let at = utc(2026, 10, 1, 14);
        let stock = spec(InstrumentKind::Equity, None, None);
        let call = spec(InstrumentKind::Call, Some("ATM"), Some("+30d"));
        let positions = resolve(&[stock, call], &m, at).unwrap();
        assert_relative_eq!(positions[0].entry_price, 600.0);

        let Instrument::Option(c) = &positions[1].instrument else {
            return;
        };
        let inputs = BsInputs::new(
            600.0,
            c.strike,
            year_fraction(at, c.expiry_datetime()),
            0.04,
            0.01,
            0.2,
        )
        .unwrap();
        assert_relative_eq!(
            positions[1].entry_price,
            pricing::price(c.option_type, &inputs)
        );
        assert!(positions[1].entry_price > 0.0);
    }

    #[test]
    fn blank_entry_price_uses_the_iv_override() {
        let m = market(600.0);
        let at = utc(2026, 10, 1, 14);
        let plain = spec(InstrumentKind::Call, Some("ATM"), Some("+30d"));
        let mut hot = plain.clone();
        hot.iv = Some(0.6);
        let p = resolve(&[plain, hot], &m, at).unwrap();
        assert!(p[1].entry_price > p[0].entry_price * 2.0);
        assert_eq!(p[1].iv_override, Some(0.6));
    }

    #[test]
    fn explicit_entry_price_is_kept() {
        let mut s = spec(InstrumentKind::Call, Some("ATM"), Some("+30d"));
        s.entry_price = Some(7.5);
        let p = resolve(&[s], &market(600.0), utc(2026, 10, 1, 14)).unwrap();
        assert_relative_eq!(p[0].entry_price, 7.5);
    }

    #[test]
    fn expiring_today_option_is_priced_at_intrinsic_after_the_close() {
        // 22:00 UTC is 18:00 New York in October, after the 16:00 expiry.
        let mut s = spec(InstrumentKind::Call, Some("570"), Some("+0d"));
        s.quantity = 2.0;
        let p = resolve(&[s], &market(600.0), utc(2026, 10, 1, 22)).unwrap();
        assert_relative_eq!(p[0].entry_price, 30.0, max_relative = 1e-9);
    }

    #[test]
    fn missing_underlying_names_the_position() {
        let mut s = spec(InstrumentKind::Equity, None, None);
        s.underlying = Symbol::new("TSLA").unwrap();
        let e = resolve(&[s], &market(600.0), utc(2026, 10, 1, 14)).unwrap_err();
        assert!(
            matches!(&e, PortfolioError::MissingUnderlying { symbol, .. } if symbol.as_str() == "TSLA")
        );
        assert!(e.to_string().contains("TSLA"));
    }

    #[test]
    fn a_strike_that_rounds_to_zero_is_rejected() {
        let e = resolve(
            &[spec(InstrumentKind::Put, Some("ATM-10%"), Some("+30d"))],
            &market(0.4),
            utc(2026, 10, 1, 14),
        )
        .unwrap_err();
        assert!(matches!(e, PortfolioError::InvalidStrike { .. }), "{e:?}");
    }

    #[test]
    fn option_without_strike_or_expiry_is_an_error() {
        let m = market(600.0);
        let at = utc(2026, 10, 1, 14);
        let no_strike = spec(InstrumentKind::Call, None, Some("+7d"));
        let no_expiry = spec(InstrumentKind::Call, Some("ATM"), None);
        assert!(matches!(
            resolve(&[no_strike], &m, at),
            Err(PortfolioError::MissingField {
                field: "strike",
                ..
            })
        ));
        assert!(matches!(
            resolve(&[no_expiry], &m, at),
            Err(PortfolioError::MissingField {
                field: "expiry",
                ..
            })
        ));
    }
}
