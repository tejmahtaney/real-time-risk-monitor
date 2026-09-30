//! Backtest result types and statistics (Part 20): one [`DailyRow`] per replayed date, the
//! [`BacktestSummary`] over them, and the Kupiec proportion-of-failures test for the `VaR` back-test.

use chrono::NaiveDate;
use pricing::normal;
use serde::{Deserialize, Serialize};

use crate::pnl::PnlBuckets;

/// The confidence level's tail probability: a 99 % `VaR` is exceeded on 1 % of days.
pub const VAR_EXCEPTION_PROBABILITY: f64 = 0.01;

/// Below this many expected exceptions (n·p) the Kupiec test has very little power.
const LOW_POWER_EXPECTED_EXCEPTIONS: f64 = 5.0;

/// One replayed date's results: the day's P&L buckets, the running total, the start-of-day
/// `VaR` and the limit breaches at the close.
///
/// `var_99` is the headline 99 % one-day `VaR` **at the start of the day**, that is, the figure
/// computed at the previous close from returns up to the day before. `exception` is true when
/// the day's loss exceeded it. Written one row per line to `daily.csv`, so the fields are flat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct DailyRow {
    /// The date replayed.
    pub date: NaiveDate,
    /// Δ·dS·scale for the day.
    pub delta: f64,
    /// ½·Γ·dS²·scale for the day.
    pub gamma: f64,
    /// ν·dσ·scale for the day.
    pub vega: f64,
    /// Θ·dt·scale for the day.
    pub theta: f64,
    /// ρ·dr·scale for the day.
    pub rho: f64,
    /// The cross and vol-convexity terms for the day.
    pub higher_order: f64,
    /// Dividend cash for the day.
    pub dividends: f64,
    /// `actual` minus the modelled buckets. Larger than in live mode, because a whole day is one step.
    pub residual: f64,
    /// The day's true change in value.
    pub actual: f64,
    /// The sum of `actual` from the first replayed date to this one.
    pub cumulative_pnl: f64,
    /// The headline 99 % one-day `VaR` at the start of the day, a positive number for a loss.
    pub var_99: f64,
    /// Whether the day's loss exceeded `var_99`.
    pub exception: bool,
    /// How many limits were in breach at the close.
    pub breaches: u32,
}

impl DailyRow {
    /// Builds a row from a day's buckets. `previous_cumulative` is the running total before the
    /// day; the exception flag comes from [`is_exception`].
    pub fn new(
        date: NaiveDate,
        buckets: PnlBuckets,
        previous_cumulative: f64,
        var_99: f64,
        breaches: u32,
    ) -> Self {
        Self {
            date,
            delta: buckets.delta,
            gamma: buckets.gamma,
            vega: buckets.vega,
            theta: buckets.theta,
            rho: buckets.rho,
            higher_order: buckets.higher_order,
            dividends: buckets.dividends,
            residual: buckets.residual,
            actual: buckets.actual,
            cumulative_pnl: previous_cumulative + buckets.actual,
            var_99,
            exception: is_exception(buckets.actual, var_99),
            breaches,
        }
    }

    /// The row's buckets.
    pub fn buckets(&self) -> PnlBuckets {
        PnlBuckets {
            delta: self.delta,
            gamma: self.gamma,
            vega: self.vega,
            theta: self.theta,
            rho: self.rho,
            higher_order: self.higher_order,
            dividends: self.dividends,
            residual: self.residual,
            actual: self.actual,
        }
    }
}

/// Whether a day with P&L `actual` breached a start-of-day `var_99`: the loss is strictly
/// greater than the `VaR`. A non-finite `VaR` never counts.
pub fn is_exception(actual: f64, var_99: f64) -> bool {
    var_99.is_finite() && -actual > var_99
}

/// The Kupiec proportion-of-failures test of a `VaR` model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct KupiecTest {
    /// Days tested.
    pub n: usize,
    /// Days whose loss exceeded the `VaR`.
    pub exceptions: usize,
    /// Exceptions expected at the model's confidence level: n × 0.01.
    pub expected_exceptions: f64,
    /// The likelihood-ratio statistic, chi-square with one degree of freedom under the null.
    pub lr: f64,
    /// The probability of an LR at least this large if the model is right.
    pub p_value: f64,
    /// Whether the model is rejected at the 5 % level (`p_value` below 0.05).
    pub reject_5pct: bool,
    /// True when fewer than 5 exceptions are expected, so the test can hardly reject anything.
    pub low_power: bool,
}

/// `x · ln(y)` with the convention 0 · ln 0 = 0.
fn xlogy(x: f64, y: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x * y.ln() }
}

/// The Kupiec proportion-of-failures test.
///
/// With `p` the model's exception probability, `n` days and `x` exceptions:
///
/// LR = −2·ln[(1 − p)^(n − x)·p^x] + 2·ln[(1 − x/n)^(n − x)·(x/n)^x]
///
/// Under the null that the model is right, LR is chi-square with one degree of freedom, so the
/// p-value is P(χ²₁ > LR) = erfc(√(LR/2)) = 2·Φ(−√LR). The zero cases (x = 0 and x = n) use
/// 0 · ln 0 = 0. Returns `None` for `n = 0`, `x > n` or `p` outside (0, 1).
///
/// ```
/// use risk::kupiec_pof;
///
/// let test = kupiec_pof(250, 5, 0.01).unwrap();
/// assert!((test.lr - 1.956809788230622).abs() < 1e-9);
/// ```
pub fn kupiec_pof(n: usize, x: usize, p: f64) -> Option<KupiecTest> {
    if n == 0 || x > n || !(p > 0.0 && p < 1.0) {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let (nf, xf) = (n as f64, x as f64);
    let observed = xf / nf;
    let log_null = xlogy(nf - xf, 1.0 - p) + xlogy(xf, p);
    let log_alt = xlogy(nf - xf, 1.0 - observed) + xlogy(xf, observed);
    // Round-off can leave a mathematically zero statistic a hair below zero.
    let lr = (2.0 * (log_alt - log_null)).max(0.0);
    let p_value = (2.0 * normal::cdf(-lr.sqrt())).clamp(0.0, 1.0);
    let expected = nf * p;
    Some(KupiecTest {
        n,
        exceptions: x,
        expected_exceptions: expected,
        lr,
        p_value,
        reject_5pct: p_value < 0.05,
        low_power: expected < LOW_POWER_EXPECTED_EXCEPTIONS,
    })
}

/// A day and its P&L.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct DayPnl {
    /// The date.
    pub date: NaiveDate,
    /// The day's actual P&L.
    pub pnl: f64,
}

/// The largest peak-to-trough fall of the cumulative P&L.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Drawdown {
    /// The fall in dollars, zero or positive.
    pub amount: f64,
    /// The date of the peak, or `None` when the peak is the start of the replay (before the first
    /// row, when cumulative P&L is zero) or there was no drawdown.
    pub peak_date: Option<NaiveDate>,
    /// The date of the trough, or `None` when there was no drawdown.
    pub trough_date: Option<NaiveDate>,
}

/// Everything a finished (or partial) backtest reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestSummary {
    /// Replayed dates so far.
    pub days: usize,
    /// The first replayed date.
    pub first_date: Option<NaiveDate>,
    /// The last replayed date so far.
    pub last_date: Option<NaiveDate>,
    /// Total P&L by bucket.
    pub total: PnlBuckets,
    /// The maximum drawdown of the cumulative P&L, counting the start as zero.
    pub max_drawdown: Drawdown,
    /// The best day by actual P&L.
    pub best_day: Option<DayPnl>,
    /// The worst day by actual P&L.
    pub worst_day: Option<DayPnl>,
    /// Days with at least one limit in breach at the close.
    pub days_with_breaches: usize,
    /// The `VaR` back-test: exceptions against expected and the Kupiec test. `None` for no days.
    pub var_backtest: Option<KupiecTest>,
}

/// Summarises `rows`, oldest first.
pub fn summarise(rows: &[DailyRow]) -> BacktestSummary {
    let mut total = PnlBuckets::default();
    let mut best: Option<DayPnl> = None;
    let mut worst: Option<DayPnl> = None;
    let mut days_with_breaches = 0;
    let mut exceptions = 0;
    let mut drawdown = Drawdown::default();
    let mut peak = 0.0_f64;
    let mut peak_date: Option<NaiveDate> = None;
    for row in rows {
        total += row.buckets();
        let day = DayPnl {
            date: row.date,
            pnl: row.actual,
        };
        if best.is_none_or(|b| day.pnl > b.pnl) {
            best = Some(day);
        }
        if worst.is_none_or(|w| day.pnl < w.pnl) {
            worst = Some(day);
        }
        if row.breaches > 0 {
            days_with_breaches += 1;
        }
        if row.exception {
            exceptions += 1;
        }
        if row.cumulative_pnl > peak {
            peak = row.cumulative_pnl;
            peak_date = Some(row.date);
        }
        let fall = peak - row.cumulative_pnl;
        if fall > drawdown.amount {
            drawdown = Drawdown {
                amount: fall,
                peak_date,
                trough_date: Some(row.date),
            };
        }
    }
    BacktestSummary {
        days: rows.len(),
        first_date: rows.first().map(|r| r.date),
        last_date: rows.last().map(|r| r.date),
        total,
        max_drawdown: drawdown,
        best_day: best,
        worst_day: worst,
        days_with_breaches,
        var_backtest: kupiec_pof(rows.len(), exceptions, VAR_EXCEPTION_PROBABILITY),
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    fn row(d: u32, actual: f64, previous: f64, var_99: f64, breaches: u32) -> DailyRow {
        let buckets = PnlBuckets {
            actual,
            residual: actual,
            ..PnlBuckets::default()
        };
        DailyRow::new(date(d), buckets, previous, var_99, breaches)
    }

    // Reference values computed independently with Python's math.log and math.erfc.
    #[test]
    fn kupiec_matches_reference_values() {
        let cases = [
            (250, 0, 5.025_167_926_750_726, 0.024_981_503_053_449_705),
            (250, 2, 0.108_435_216_236_799_19, 0.741_932_700_926_281),
            (250, 5, 1.956_809_788_230_622, 0.161_854_917_196_042_72),
            (250, 10, 12.955_491_062_356_018, 0.000_318_984_508_213_38),
            (100, 0, 2.010_067_170_700_29, 0.156_258_399_534_850_85),
            (60, 3, 4.954_930_201_722_998, 0.026_016_375_107_772_875),
            (60, 0, 1.206_040_302_420_174, 0.272_117_750_132_306_2),
        ];
        for (n, x, lr, p_value) in cases {
            let test = kupiec_pof(n, x, 0.01).unwrap();
            assert_relative_eq!(test.lr, lr, max_relative = 1e-9);
            assert_relative_eq!(test.p_value, p_value, max_relative = 1e-6);
            assert_eq!(test.reject_5pct, p_value < 0.05);
        }
    }

    #[test]
    fn kupiec_handles_no_exceptions_and_all_exceptions() {
        // x = 0: LR = -2 n ln(1 - p).
        let none = kupiec_pof(100, 0, 0.01).unwrap();
        assert_relative_eq!(none.lr, -200.0 * 0.99_f64.ln(), max_relative = 1e-12);
        // x = n: LR = -2 n ln p, a hopeless model.
        let all = kupiec_pof(50, 50, 0.01).unwrap();
        assert_relative_eq!(all.lr, -100.0 * 0.01_f64.ln(), max_relative = 1e-12);
        assert!(all.p_value < 1e-100);
        assert!(all.reject_5pct);
        // The observed rate equals p exactly: no evidence against the model.
        let exact = kupiec_pof(500, 5, 0.01).unwrap();
        assert_relative_eq!(exact.lr, 0.0, epsilon = 1e-12);
        assert_relative_eq!(exact.p_value, 1.0, epsilon = 1e-9);
    }

    #[test]
    fn kupiec_rejects_bad_inputs() {
        assert!(kupiec_pof(0, 0, 0.01).is_none());
        assert!(kupiec_pof(10, 11, 0.01).is_none());
        assert!(kupiec_pof(10, 1, 0.0).is_none());
        assert!(kupiec_pof(10, 1, 1.0).is_none());
    }

    #[test]
    fn a_short_sample_is_flagged_as_low_power() {
        let short = kupiec_pof(60, 0, 0.01).unwrap();
        assert!(short.low_power);
        assert_relative_eq!(short.expected_exceptions, 0.6);
        assert!(!short.reject_5pct);
        assert!(!kupiec_pof(500, 5, 0.01).unwrap().low_power);
    }

    #[test]
    fn an_exception_needs_a_loss_strictly_above_the_var() {
        assert!(is_exception(-101.0, 100.0));
        assert!(!is_exception(-100.0, 100.0));
        assert!(!is_exception(50.0, 100.0));
        assert!(!is_exception(-1.0, f64::NAN));
    }

    #[test]
    fn the_summary_finds_drawdown_best_and_worst_days() {
        // Cumulative: 10, 30, 5, -15, 5.
        let mut rows = vec![row(2, 10.0, 0.0, 100.0, 0)];
        for (d, pnl, breaches) in [(3, 20.0, 1), (4, -25.0, 2), (5, -20.0, 0), (6, 20.0, 0)] {
            let previous = rows.last().map_or(0.0, |r| r.cumulative_pnl);
            rows.push(row(d, pnl, previous, 100.0, breaches));
        }
        let summary = summarise(&rows);
        assert_eq!(summary.days, 5);
        assert_eq!(summary.first_date, Some(date(2)));
        assert_eq!(summary.last_date, Some(date(6)));
        assert_relative_eq!(summary.total.actual, 5.0);
        assert_relative_eq!(summary.max_drawdown.amount, 45.0);
        assert_eq!(summary.max_drawdown.peak_date, Some(date(3)));
        assert_eq!(summary.max_drawdown.trough_date, Some(date(5)));
        assert_eq!(summary.best_day.unwrap().date, date(3));
        assert_eq!(summary.worst_day.unwrap().date, date(4));
        assert_eq!(summary.days_with_breaches, 2);
        assert_eq!(summary.var_backtest.unwrap().exceptions, 0);
    }

    #[test]
    fn a_drawdown_from_the_start_has_no_peak_date() {
        let first = row(2, -10.0, 0.0, 100.0, 0);
        let second = row(3, 4.0, first.cumulative_pnl, 100.0, 0);
        let summary = summarise(&[first, second]);
        assert_relative_eq!(summary.max_drawdown.amount, 10.0);
        assert_eq!(summary.max_drawdown.peak_date, None);
        assert_eq!(summary.max_drawdown.trough_date, Some(date(2)));
    }

    #[test]
    fn exceptions_are_counted_against_the_start_of_day_var() {
        let rows = vec![
            row(2, -150.0, 0.0, 100.0, 0),
            row(3, -50.0, -150.0, 100.0, 0),
        ];
        assert!(rows[0].exception);
        assert!(!rows[1].exception);
        let test = summarise(&rows).var_backtest.unwrap();
        assert_eq!((test.n, test.exceptions), (2, 1));
    }

    #[test]
    fn an_empty_replay_summarises_to_nothing() {
        let summary = summarise(&[]);
        assert_eq!(summary.days, 0);
        assert!(summary.var_backtest.is_none());
        assert!(summary.best_day.is_none());
    }
}
