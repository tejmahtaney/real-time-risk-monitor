//! Instruments: equities and European equity options.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use pricing::OptionType;
use serde::{Deserialize, Serialize};

use crate::error::{DomainError, finite};
use crate::symbol::Symbol;
use crate::time::MARKET_TZ;

/// Standard equity option contract multiplier: shares per contract.
pub const DEFAULT_MULTIPLIER: f64 = 100.0;

fn default_multiplier() -> f64 {
    DEFAULT_MULTIPLIER
}

/// The broad kind of an instrument. Shared with the `kind` column of the portfolio CSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
pub enum InstrumentKind {
    /// Shares of the underlying.
    Equity,
    /// A European call option.
    Call,
    /// A European put option.
    Put,
}

/// A European option on an equity.
///
/// The multiplier is `f64` because a split adjustment (Part 19) can make it
/// 150. The fields are public and plain data. Pricing revalidates them, so a
/// bad contract shows up as a typed error there and not as a NaN.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionContract {
    /// The underlying equity.
    pub underlying: Symbol,
    /// Call or put.
    pub option_type: OptionType,
    /// Strike price in dollars.
    pub strike: f64,
    /// Expiry date. The option expires at 16:00 New York time on this date.
    pub expiry: NaiveDate,
    /// Shares per contract. Defaults to 100.
    #[serde(default = "default_multiplier")]
    pub multiplier: f64,
}

impl OptionContract {
    /// Builds a contract with the standard multiplier of 100.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] or [`DomainError::InvalidStrike`] when the
    /// strike is not a finite positive number.
    ///
    /// ```
    /// use chrono::NaiveDate;
    /// use domain::{OptionContract, OptionType, Symbol};
    ///
    /// let expiry = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    /// let c = OptionContract::new(Symbol::new("AAPL")?, OptionType::Call, 230.0, expiry)?;
    /// assert_eq!(c.multiplier, 100.0);
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(
        underlying: Symbol,
        option_type: OptionType,
        strike: f64,
        expiry: NaiveDate,
    ) -> Result<Self, DomainError> {
        finite("strike", strike)?;
        if strike <= 0.0 {
            return Err(DomainError::InvalidStrike(strike));
        }
        Ok(Self {
            underlying,
            option_type,
            strike,
            expiry,
            multiplier: DEFAULT_MULTIPLIER,
        })
    }

    /// Returns the contract with a different multiplier, for example 150 after a 3-for-2 split.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] or [`DomainError::InvalidMultiplier`] when the
    /// multiplier is not a finite positive number.
    pub fn with_multiplier(mut self, multiplier: f64) -> Result<Self, DomainError> {
        finite("multiplier", multiplier)?;
        if multiplier <= 0.0 {
            return Err(DomainError::InvalidMultiplier(multiplier));
        }
        self.multiplier = multiplier;
        Ok(self)
    }

    /// The moment the option expires, as UTC: 16:00 `America/New_York` on the expiry date.
    ///
    /// The zone comes from `chrono-tz`, so daylight saving is handled: 20:00 UTC
    /// in summer (EDT) and 21:00 UTC in winter (EST).
    ///
    /// ```
    /// use chrono::{NaiveDate, TimeZone, Utc};
    /// use domain::{OptionContract, OptionType, Symbol};
    ///
    /// let summer = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
    /// let c = OptionContract::new(Symbol::new("SPY")?, OptionType::Put, 500.0, summer)?;
    /// assert_eq!(c.expiry_datetime(), Utc.with_ymd_and_hms(2026, 7, 17, 20, 0, 0).unwrap());
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn expiry_datetime(&self) -> DateTime<Utc> {
        let close = NaiveTime::from_hms_opt(16, 0, 0).unwrap_or(NaiveTime::MIN);
        match MARKET_TZ
            .from_local_datetime(&self.expiry.and_time(close))
            .earliest()
        {
            Some(local) => local.with_timezone(&Utc),
            // Unreachable: New York changes its clocks at 02:00, so 16:00 always
            // exists exactly once. Fall back to 21:00 UTC (16:00 EST) without panicking.
            None => (self.expiry.and_time(close) + chrono::Duration::hours(5)).and_utc(),
        }
    }
}

/// Something that can be held in a position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Instrument {
    /// Shares of an equity.
    Equity {
        /// The ticker.
        symbol: Symbol,
    },
    /// A European option.
    Option(OptionContract),
}

impl Instrument {
    /// The underlying equity. For shares that is the shares' own symbol.
    pub fn underlying(&self) -> &Symbol {
        match self {
            Self::Equity { symbol } => symbol,
            Self::Option(contract) => &contract.underlying,
        }
    }

    /// Equity, call or put.
    pub fn kind(&self) -> InstrumentKind {
        match self {
            Self::Equity { .. } => InstrumentKind::Equity,
            Self::Option(c) => match c.option_type {
                OptionType::Call => InstrumentKind::Call,
                OptionType::Put => InstrumentKind::Put,
            },
        }
    }

    /// Units per position quantity: 1 for shares, the contract multiplier for options.
    pub fn multiplier(&self) -> f64 {
        match self {
            Self::Equity { .. } => 1.0,
            Self::Option(c) => c.multiplier,
        }
    }

    /// A short description such as `AAPL 2026-10-16 C 230`, or just the ticker for shares.
    ///
    /// ```
    /// use chrono::NaiveDate;
    /// use domain::{Instrument, OptionContract, OptionType, Symbol};
    ///
    /// let expiry = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
    /// let call = OptionContract::new(Symbol::new("AAPL")?, OptionType::Call, 230.0, expiry)?;
    /// assert_eq!(Instrument::Option(call).description(), "AAPL 2026-10-16 C 230");
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn description(&self) -> String {
        match self {
            Self::Equity { symbol } => symbol.to_string(),
            Self::Option(c) => {
                let letter = match c.option_type {
                    OptionType::Call => 'C',
                    OptionType::Put => 'P',
                };
                format!(
                    "{} {} {} {}",
                    c.underlying,
                    c.expiry,
                    letter,
                    format_strike(c.strike)
                )
            }
        }
    }
}

/// Formats a strike with at most two decimals and no trailing zeros, for display only.
fn format_strike(strike: f64) -> String {
    let text = format!("{strike:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn contract(kind: OptionType, strike: f64, expiry: NaiveDate) -> OptionContract {
        OptionContract::new(sym("AAPL"), kind, strike, expiry).unwrap()
    }

    #[test]
    fn multiplier_defaults_to_100() {
        let c = contract(OptionType::Call, 230.0, date(2026, 10, 16));
        assert_eq!(c.multiplier.to_bits(), 100.0_f64.to_bits());
        let json =
            r#"{"underlying":"AAPL","option_type":"Call","strike":230.0,"expiry":"2026-10-16"}"#;
        let parsed: OptionContract = serde_json::from_str(json).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn contract_validation() {
        let e = date(2026, 10, 16);
        assert!(OptionContract::new(sym("A"), OptionType::Call, 0.0, e).is_err());
        assert!(OptionContract::new(sym("A"), OptionType::Call, -5.0, e).is_err());
        assert!(OptionContract::new(sym("A"), OptionType::Call, f64::NAN, e).is_err());
        let c = contract(OptionType::Put, 100.0, e);
        assert!(c.clone().with_multiplier(0.0).is_err());
        assert!(c.clone().with_multiplier(f64::INFINITY).is_err());
        assert_eq!(
            c.with_multiplier(150.0).unwrap().multiplier.to_bits(),
            150.0_f64.to_bits()
        );
    }

    #[test]
    fn expiry_is_2000_utc_in_summer() {
        let c = contract(OptionType::Call, 100.0, date(2026, 7, 17));
        assert_eq!(
            c.expiry_datetime(),
            Utc.with_ymd_and_hms(2026, 7, 17, 20, 0, 0).unwrap()
        );
    }

    #[test]
    fn expiry_is_2100_utc_in_winter() {
        let c = contract(OptionType::Call, 100.0, date(2026, 12, 18));
        assert_eq!(
            c.expiry_datetime(),
            Utc.with_ymd_and_hms(2026, 12, 18, 21, 0, 0).unwrap()
        );
    }

    #[test]
    fn expiry_tracks_the_daylight_saving_changeover() {
        // US clocks go forward on 8 March 2026 and back on 1 November 2026.
        let hour = |y, m, d| {
            contract(OptionType::Call, 100.0, date(y, m, d))
                .expiry_datetime()
                .hour()
        };
        assert_eq!(hour(2026, 3, 6), 21);
        assert_eq!(hour(2026, 3, 9), 20);
        assert_eq!(hour(2026, 10, 30), 20);
        assert_eq!(hour(2026, 11, 2), 21);
    }

    #[test]
    fn expiry_is_always_1600_in_new_york() {
        let mut d = date(2025, 1, 1);
        while d < date(2029, 1, 1) {
            let utc = contract(OptionType::Call, 100.0, d).expiry_datetime();
            assert!(matches!(utc.hour(), 20 | 21), "{d}: {utc}");
            let local = utc.with_timezone(&MARKET_TZ);
            assert_eq!(
                (local.date_naive(), local.hour(), local.minute()),
                (d, 16, 0),
                "{d}"
            );
            d = d.succ_opt().unwrap();
        }
    }

    #[test]
    fn descriptions() {
        let call = Instrument::Option(contract(OptionType::Call, 230.0, date(2026, 10, 16)));
        assert_eq!(call.description(), "AAPL 2026-10-16 C 230");
        let put = Instrument::Option(contract(OptionType::Put, 227.5, date(2026, 10, 16)));
        assert_eq!(put.description(), "AAPL 2026-10-16 P 227.5");
        assert_eq!(
            Instrument::Equity { symbol: sym("SPY") }.description(),
            "SPY"
        );
    }

    #[test]
    fn description_rounds_adjusted_strikes_but_keeps_exact_value() {
        let strike = 113.476_190_476_190_48;
        let option = Instrument::Option(contract(OptionType::Call, strike, date(2026, 10, 16)));
        assert_eq!(option.description(), "AAPL 2026-10-16 C 113.48");
        let Instrument::Option(c) = &option else {
            return;
        };
        assert_eq!(c.strike.to_bits(), strike.to_bits());
        let round = Instrument::Option(contract(OptionType::Put, 100.001, date(2026, 10, 16)));
        assert_eq!(round.description(), "AAPL 2026-10-16 P 100");
    }

    #[test]
    fn underlying_kind_and_multiplier() {
        let stock = Instrument::Equity {
            symbol: sym("MSFT"),
        };
        assert_eq!(stock.underlying().as_str(), "MSFT");
        assert_eq!(stock.kind(), InstrumentKind::Equity);
        assert_eq!(stock.multiplier().to_bits(), 1.0_f64.to_bits());
        let put = Instrument::Option(contract(OptionType::Put, 100.0, date(2026, 10, 16)));
        assert_eq!(put.underlying().as_str(), "AAPL");
        assert_eq!(put.kind(), InstrumentKind::Put);
        assert_eq!(put.multiplier().to_bits(), 100.0_f64.to_bits());
        let call = Instrument::Option(contract(OptionType::Call, 100.0, date(2026, 10, 16)));
        assert_eq!(call.kind(), InstrumentKind::Call);
    }

    #[test]
    fn instrument_serde_round_trip() {
        let call = Instrument::Option(contract(OptionType::Call, 230.0, date(2026, 10, 16)));
        let json = serde_json::to_string(&call).unwrap();
        assert_eq!(serde_json::from_str::<Instrument>(&json).unwrap(), call);
        let stock = Instrument::Equity { symbol: sym("SPY") };
        let json = serde_json::to_string(&stock).unwrap();
        assert_eq!(serde_json::from_str::<Instrument>(&json).unwrap(), stock);
    }
}
