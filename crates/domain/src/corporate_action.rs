//! Corporate actions: stock splits and cash dividends, and the spot arithmetic they imply.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::{DomainError, finite};
use crate::symbol::Symbol;

/// Tolerance for treating a split ratio as a whole number.
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;

/// An event that changes what a symbol's shares are or pay.
///
/// A split ratio of 4.0 is a 4-for-1 forward split, 1.5 a 3-for-2 and 0.1 a 1-for-10
/// reverse split. Both kinds take effect at the session open on their date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CorporateAction {
    /// A stock split.
    Split {
        /// The underlying that splits.
        symbol: Symbol,
        /// New shares per old share. 4.0 is 4-for-1 and 0.1 is 1-for-10.
        ratio: f64,
        /// The first session that trades post-split.
        effective: NaiveDate,
    },
    /// A cash dividend of `amount` per share.
    CashDividend {
        /// The underlying that pays.
        symbol: Symbol,
        /// Cash per share, in dollars.
        amount: f64,
        /// The first session that trades without the dividend.
        ex_date: NaiveDate,
    },
}

impl CorporateAction {
    /// Builds a split, checking the ratio.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidSplitRatio`] unless the ratio is finite, positive and not 1.
    pub fn split(symbol: Symbol, ratio: f64, effective: NaiveDate) -> Result<Self, DomainError> {
        if !ratio.is_finite() || ratio <= 0.0 || (ratio - 1.0).abs() < WHOLE_NUMBER_TOLERANCE {
            return Err(DomainError::InvalidSplitRatio(ratio));
        }
        Ok(Self::Split {
            symbol,
            ratio,
            effective,
        })
    }

    /// Builds a cash dividend, checking the amount.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidDividendAmount`] unless the amount is finite and positive.
    pub fn cash_dividend(
        symbol: Symbol,
        amount: f64,
        ex_date: NaiveDate,
    ) -> Result<Self, DomainError> {
        if !amount.is_finite() || amount <= 0.0 {
            return Err(DomainError::InvalidDividendAmount(amount));
        }
        Ok(Self::CashDividend {
            symbol,
            amount,
            ex_date,
        })
    }

    /// The underlying the action applies to.
    pub fn symbol(&self) -> &Symbol {
        match self {
            Self::Split { symbol, .. } | Self::CashDividend { symbol, .. } => symbol,
        }
    }

    /// The session date the action takes effect on.
    pub fn date(&self) -> NaiveDate {
        match self {
            Self::Split { effective, .. } => *effective,
            Self::CashDividend { ex_date, .. } => *ex_date,
        }
    }

    /// A one-line description for logs and the alerts pane, such as `AAPL 4-for-1 split`.
    pub fn describe(&self) -> String {
        match self {
            Self::Split { symbol, ratio, .. } => {
                format!("{symbol} {} split", ratio_words(*ratio))
            }
            Self::CashDividend { symbol, amount, .. } => {
                format!("{symbol} cash dividend of {amount:.4} per share")
            }
        }
    }
}

/// `4-for-1`, `3-for-2`, `1-for-10`, or the bare ratio when no small fraction fits.
fn ratio_words(ratio: f64) -> String {
    if ratio >= 1.0 {
        for old in 1..=10u32 {
            let new = ratio * f64::from(old);
            if (new - new.round()).abs() < 1e-9 {
                return format!("{}-for-{old}", new.round());
            }
        }
    } else {
        let old = 1.0 / ratio;
        if (old - old.round()).abs() < 1e-9 {
            return format!("1-for-{}", old.round());
        }
    }
    format!("{ratio}-for-1")
}

/// Whether `ratio` is a whole-number forward split such as 2, 3 or 4.
///
/// The OCC keeps the contract count in proportion for these (contracts × ratio, strike ÷
/// ratio, multiplier unchanged). Every other ratio, such as 1.5 or 0.1, keeps the number
/// of contracts and changes the multiplier instead.
///
/// ```
/// use domain::is_whole_number_forward_split;
///
/// assert!(is_whole_number_forward_split(4.0));
/// assert!(!is_whole_number_forward_split(1.5));
/// assert!(!is_whole_number_forward_split(0.1));
/// ```
pub fn is_whole_number_forward_split(ratio: f64) -> bool {
    ratio > 1.0 && (ratio - ratio.round()).abs() < WHOLE_NUMBER_TOLERANCE
}

/// A change to a spot price at a session open.
///
/// One value drives both the market state and the price simulator, so their spots stay
/// identical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpotAdjustment {
    /// Split: the spot becomes `spot / ratio`.
    DivideBy(f64),
    /// Cash dividend: the spot drops by the dividend, `spot − amount`.
    Subtract(f64),
}

impl SpotAdjustment {
    /// The adjusted spot.
    ///
    /// # Errors
    ///
    /// [`DomainError::NonFinite`] for a bad input, [`DomainError::InvalidSplitRatio`] for a
    /// non-positive ratio and [`DomainError::NonPositiveAdjustedSpot`] when the result is
    /// not above zero.
    ///
    /// ```
    /// use domain::SpotAdjustment;
    ///
    /// assert_eq!(SpotAdjustment::DivideBy(4.0).apply(200.0)?, 50.0);
    /// assert_eq!(SpotAdjustment::Subtract(0.5).apply(100.0)?, 99.5);
    /// assert!(SpotAdjustment::Subtract(100.0).apply(100.0).is_err());
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn apply(self, spot: f64) -> Result<f64, DomainError> {
        finite("spot", spot)?;
        let result = match self {
            Self::DivideBy(ratio) => {
                if !ratio.is_finite() || ratio <= 0.0 {
                    return Err(DomainError::InvalidSplitRatio(ratio));
                }
                spot / ratio
            }
            Self::Subtract(amount) => {
                finite("amount", amount)?;
                spot - amount
            }
        };
        if result.is_finite() && result > 0.0 {
            Ok(result)
        } else {
            Err(DomainError::NonPositiveAdjustedSpot { spot, result })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
    }

    #[test]
    fn constructors_validate() {
        assert!(CorporateAction::split(sym("AAPL"), 4.0, date()).is_ok());
        assert!(CorporateAction::split(sym("AAPL"), 0.1, date()).is_ok());
        for bad in [0.0, -2.0, 1.0, f64::NAN, f64::INFINITY] {
            assert!(CorporateAction::split(sym("AAPL"), bad, date()).is_err());
        }
        assert!(CorporateAction::cash_dividend(sym("MSFT"), 0.75, date()).is_ok());
        for bad in [0.0, -0.1, f64::NAN] {
            assert!(CorporateAction::cash_dividend(sym("MSFT"), bad, date()).is_err());
        }
    }

    #[test]
    fn whole_number_forward_rule() {
        assert!(is_whole_number_forward_split(2.0));
        assert!(is_whole_number_forward_split(4.0));
        assert!(!is_whole_number_forward_split(1.5));
        assert!(!is_whole_number_forward_split(1.0));
        assert!(!is_whole_number_forward_split(0.5));
        assert!(!is_whole_number_forward_split(0.1));
    }

    #[test]
    fn descriptions_read_like_a_split_announcement() {
        let split = |r| {
            CorporateAction::split(sym("AAPL"), r, date())
                .unwrap()
                .describe()
        };
        assert_eq!(split(4.0), "AAPL 4-for-1 split");
        assert_eq!(split(1.5), "AAPL 3-for-2 split");
        assert_eq!(split(0.1), "AAPL 1-for-10 split");
    }

    #[test]
    fn spot_adjustment_rejects_a_non_positive_result() {
        assert_eq!(SpotAdjustment::DivideBy(2.0).apply(10.0), Ok(5.0));
        assert!(matches!(
            SpotAdjustment::Subtract(10.0).apply(10.0),
            Err(DomainError::NonPositiveAdjustedSpot { .. })
        ));
        assert!(SpotAdjustment::DivideBy(0.0).apply(10.0).is_err());
    }

    #[test]
    fn serde_uses_a_type_tag() {
        let action = CorporateAction::split(sym("AAPL"), 4.0, date()).unwrap();
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""type":"split""#));
        assert_eq!(
            serde_json::from_str::<CorporateAction>(&json).unwrap(),
            action
        );
    }
}
