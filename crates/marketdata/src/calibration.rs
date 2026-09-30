//! Calibration: split adjustment, volatility and correlation from daily closes.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use domain::Symbol;

use crate::config::{DividendModel, MarketConfig, VolMethod};
use crate::error::CalibrationError;
use crate::loader::LoadedHistory;
use crate::seed::{SeedData, SymbolSeed};
use crate::source::DailyBar;

/// Trading days per year, used to annualise daily volatility: annual = daily × √252.
pub const TRADING_DAYS_PER_YEAR: f64 = 252.0;

/// The `RiskMetrics` decay factor for the EWMA volatility.
pub const EWMA_LAMBDA: f64 = 0.94;

/// Split ratios the detector recognises, before taking inverses for reverse splits.
const SPLIT_RATIOS: [f64; 7] = [1.5, 2.0, 3.0, 4.0, 5.0, 10.0, 20.0];

/// A day-over-day price ratio within this fraction of a split ratio counts as that split.
const SPLIT_TOLERANCE: f64 = 0.02;

/// Below this a return series is treated as constant, so its correlation is undefined.
const MIN_RETURN_VARIANCE: f64 = 1e-24;

/// A detected split: the first date at the new price and the nominal ratio, where 10.0
/// is a 10-for-1 split and 0.1 a 1-for-10 reverse split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Split {
    pub date: NaiveDate,
    pub ratio: f64,
}

/// The nominal split ratio that explains the fall from `previous` to `current`, if any.
fn detect_split(previous: f64, current: f64) -> Option<f64> {
    let observed = previous / current;
    SPLIT_RATIOS
        .iter()
        .flat_map(|&n| [n, 1.0 / n])
        .find(|&candidate| (observed / candidate - 1.0).abs() <= SPLIT_TOLERANCE)
}

/// Adjusts closes for detected splits, so every price is on the basis of the latest one.
///
/// The closes are raw as-traded, so a 10-for-1 split shows up as a −90 % "return" that
/// would swamp the volatility. Earlier closes are divided by the nominal ratio, not the
/// observed one, so the split-day return keeps the real price move. `bars` must be sorted.
pub(crate) fn adjust_splits(symbol: &Symbol, bars: &[DailyBar]) -> (Vec<DailyBar>, Vec<Split>) {
    let mut adjusted = Vec::with_capacity(bars.len());
    let mut splits = Vec::new();
    let mut factor = 1.0;
    for (i, bar) in bars.iter().enumerate().rev() {
        adjusted.push(DailyBar {
            date: bar.date,
            close: bar.close / factor,
        });
        let previous = i.checked_sub(1).and_then(|p| bars.get(p));
        if let Some(previous) = previous
            && let Some(ratio) = detect_split(previous.close, bar.close)
        {
            tracing::warn!(
                %symbol,
                date = %bar.date,
                ratio,
                before = previous.close,
                after = bar.close,
                "split detected, adjusting earlier closes"
            );
            splits.push(Split {
                date: bar.date,
                ratio,
            });
            factor *= ratio;
        }
    }
    adjusted.reverse();
    splits.reverse();
    (adjusted, splits)
}

/// Sorts, validates and returns a symbol's bars.
fn clean(symbol: &Symbol, bars: &[DailyBar]) -> Result<Vec<DailyBar>, CalibrationError> {
    let mut sorted = bars.to_vec();
    sorted.sort_by_key(|b| b.date);
    for bar in &sorted {
        if !bar.close.is_finite() || bar.close <= 0.0 {
            return Err(CalibrationError::InvalidClose {
                symbol: symbol.clone(),
                date: bar.date,
                value: bar.close,
            });
        }
    }
    let repeated = sorted.windows(2).find_map(|w| match w {
        [a, b] if a.date == b.date => Some(a.date),
        _ => None,
    });
    if let Some(date) = repeated {
        return Err(CalibrationError::DuplicateDate {
            symbol: symbol.clone(),
            date,
        });
    }
    Ok(sorted)
}

/// Log returns ln(Cₜ / Cₜ₋₁) of consecutive closes.
pub(crate) fn log_returns(closes: &[f64]) -> Vec<f64> {
    closes
        .windows(2)
        .filter_map(|w| match w {
            [previous, current] => Some((current / previous).ln()),
            _ => None,
        })
        .collect()
}

/// Realised volatility: sample standard deviation (n − 1) times √252.
pub(crate) fn realised_vol(returns: &[f64]) -> f64 {
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (variance * TRADING_DAYS_PER_YEAR).sqrt()
}

/// The `RiskMetrics` variance after the last return: σ²ₜ = λσ²ₜ₋₁ + (1 − λ)r²ₜ₋₁,
/// started from the first squared return.
fn ewma_variance(returns: &[f64]) -> f64 {
    let mut iter = returns.iter();
    let Some(first) = iter.next() else {
        return 0.0;
    };
    iter.fold(first * first, |variance, r| {
        EWMA_LAMBDA * variance + (1.0 - EWMA_LAMBDA) * r * r
    })
}

/// EWMA volatility: `RiskMetrics` recursion with λ = 0.94, times √252.
pub(crate) fn ewma_vol(returns: &[f64]) -> f64 {
    (ewma_variance(returns) * TRADING_DAYS_PER_YEAR).sqrt()
}

/// Pearson correlation matrix of the columns. The diagonal is exactly 1, each pair is
/// computed once in a fixed order so the matrix is exactly symmetric, and values are
/// clamped to [−1, 1] against rounding.
pub(crate) fn correlation_matrix(
    symbols: &[Symbol],
    columns: &[Vec<f64>],
) -> Result<Vec<Vec<f64>>, CalibrationError> {
    let centred: Vec<Vec<f64>> = columns
        .iter()
        .map(|col| {
            let mean = col.iter().sum::<f64>() / col.len() as f64;
            col.iter().map(|x| x - mean).collect()
        })
        .collect();
    let sums: Vec<f64> = centred
        .iter()
        .map(|col| col.iter().map(|x| x * x).sum::<f64>())
        .collect();
    for ((symbol, column), sum) in symbols.iter().zip(columns).zip(&sums) {
        let variance = sum / (column.len() as f64 - 1.0);
        if !variance.is_finite() || variance <= MIN_RETURN_VARIANCE {
            return Err(CalibrationError::ZeroVariance(symbol.clone()));
        }
    }
    let rho = |(a, sum_a): (&Vec<f64>, &f64), (b, sum_b): (&Vec<f64>, &f64)| {
        let cross: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        (cross / (sum_a * sum_b).sqrt()).clamp(-1.0, 1.0)
    };
    let items: Vec<(&Vec<f64>, &f64)> = centred.iter().zip(&sums).collect();
    Ok(items
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            items
                .iter()
                .enumerate()
                .map(|(j, &b)| match i.cmp(&j) {
                    Ordering::Equal => 1.0,
                    Ordering::Less => rho(a, b),
                    Ordering::Greater => rho(b, a),
                })
                .collect()
        })
        .collect())
}

/// Calibrates start prices, volatilities and correlations from daily closes.
///
/// For each configured symbol: sort the closes, adjust for splits, take log returns
/// ln(Cₜ/Cₜ₋₁), and over the last `vol_window_days` returns compute the realised vol
/// (sample standard deviation with n − 1, times √252) and the EWMA vol (`RiskMetrics`,
/// λ = 0.94, times √252). The spot is the last close. `vol_method` picks which vol seeds
/// the simulator. Correlations and the return matrix come from closes inner-joined by
/// date across all symbols, over the last `vol_window_days` aligned returns. Each
/// symbol's vol uses its own series, so a date missing for one symbol does not shorten
/// the others'.
///
/// # Errors
///
/// [`CalibrationError`] for a missing history or dividend yield, an invalid or repeated
/// close, too little history or date overlap, a zero-variance series, or an unusable vol.
pub fn calibrate(
    histories: &BTreeMap<Symbol, LoadedHistory>,
    config: &MarketConfig,
) -> Result<SeedData, CalibrationError> {
    if histories.is_empty() {
        return Err(CalibrationError::NoHistories);
    }
    let window = config.vol_window_days;
    let mut underlyings = BTreeMap::new();
    let mut adjusted_all: Vec<Vec<DailyBar>> = Vec::with_capacity(config.symbols.len());
    for symbol in &config.symbols {
        let history = histories
            .get(symbol)
            .ok_or_else(|| CalibrationError::MissingHistory(symbol.clone()))?;
        let dividend_yield = *config
            .dividend_yields
            .get(symbol)
            .ok_or_else(|| CalibrationError::MissingDividendYield(symbol.clone()))?;
        // A symbol on the discrete model has q = 0: its dividends are counted once, as cash.
        let dividend_yield = match config.model_of(symbol) {
            DividendModel::Yield => dividend_yield,
            DividendModel::Discrete => 0.0,
        };
        let (seed, adjusted) = calibrate_symbol(symbol, history, dividend_yield, config)?;
        underlyings.insert(symbol.clone(), seed);
        adjusted_all.push(adjusted);
    }

    // Latest close date across the symbols.
    let last_dates: Vec<NaiveDate> = adjusted_all
        .iter()
        .filter_map(|bars| bars.last().map(|b| b.date))
        .collect();
    let as_of = last_dates
        .iter()
        .copied()
        .max()
        .ok_or(CalibrationError::NoHistories)?;
    if last_dates.iter().any(|d| *d != as_of) {
        tracing::warn!(?last_dates, %as_of, "symbols end on different dates");
    }

    let (return_dates, columns) = aligned_returns(&adjusted_all, window)?;
    let correlation = correlation_matrix(&config.symbols, &columns)?;
    // Transpose the columns into one row per date.
    let mut cursors: Vec<_> = columns.iter().map(|col| col.iter().copied()).collect();
    let returns: Vec<Vec<f64>> = (0..window)
        .map(|_| cursors.iter_mut().filter_map(Iterator::next).collect())
        .collect();

    Ok(SeedData {
        as_of,
        rate: config.risk_free_rate,
        symbols: config.symbols.clone(),
        underlyings,
        return_dates,
        returns,
        correlation,
        dividend_schedule: BTreeMap::new(),
    })
}

/// One symbol's seed and its split-adjusted bars.
fn calibrate_symbol(
    symbol: &Symbol,
    history: &LoadedHistory,
    dividend_yield: f64,
    config: &MarketConfig,
) -> Result<(SymbolSeed, Vec<DailyBar>), CalibrationError> {
    let window = config.vol_window_days;
    let bars = clean(symbol, &history.bars)?;
    if bars.len() < window + 1 {
        return Err(CalibrationError::InsufficientHistory {
            symbol: symbol.clone(),
            have: bars.len(),
            need: window + 1,
            window,
        });
    }
    let (adjusted, _splits) = adjust_splits(symbol, &bars);
    let closes: Vec<f64> = adjusted.iter().map(|b| b.close).collect();
    let returns = log_returns(&closes);
    let recent = returns
        .get(returns.len().saturating_sub(window)..)
        .ok_or_else(|| CalibrationError::InsufficientHistory {
            symbol: symbol.clone(),
            have: bars.len(),
            need: window + 1,
            window,
        })?;
    let vol_realised = realised_vol(recent);
    let vol_ewma = ewma_vol(recent);
    for value in [vol_realised, vol_ewma] {
        if !value.is_finite() || value <= 0.0 {
            return Err(CalibrationError::BadVol {
                symbol: symbol.clone(),
                value,
            });
        }
    }
    let vol = match config.vol_method {
        VolMethod::Realised => vol_realised,
        VolMethod::Ewma => vol_ewma,
    };
    let spot = closes
        .last()
        .copied()
        .ok_or_else(|| CalibrationError::MissingHistory(symbol.clone()))?;
    let seed = SymbolSeed {
        spot,
        vol_realised,
        vol_ewma,
        vol,
        dividend_yield,
        source: history.source.clone(),
    };
    Ok((seed, adjusted))
}

/// Inner-joins the symbols' closes by date and returns the dates of the last `window`
/// aligned returns with one column of log returns per symbol.
fn aligned_returns(
    adjusted_all: &[Vec<DailyBar>],
    window: usize,
) -> Result<(Vec<NaiveDate>, Vec<Vec<f64>>), CalibrationError> {
    let by_date: Vec<BTreeMap<NaiveDate, f64>> = adjusted_all
        .iter()
        .map(|bars| bars.iter().map(|b| (b.date, b.close)).collect())
        .collect();
    let common: BTreeSet<NaiveDate> = by_date
        .first()
        .map(|first| {
            first
                .keys()
                .copied()
                .filter(|d| by_date.iter().all(|m| m.contains_key(d)))
                .collect()
        })
        .unwrap_or_default();
    if common.len() < window + 1 {
        return Err(CalibrationError::InsufficientOverlap {
            have: common.len().saturating_sub(1),
            need: window,
        });
    }
    let all_dates: Vec<NaiveDate> = common.into_iter().collect();
    let dates: Vec<NaiveDate> = all_dates
        .iter()
        .skip(all_dates.len() - (window + 1))
        .copied()
        .collect();
    let columns = by_date
        .iter()
        .map(|map| {
            let closes: Vec<f64> = dates.iter().filter_map(|d| map.get(d).copied()).collect();
            log_returns(&closes)
        })
        .collect();
    Ok((dates.iter().skip(1).copied().collect(), columns))
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::{Duration, TimeZone, Utc};

    use super::*;
    use crate::config::DataMode;
    use crate::loader::DataSource;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn day(i: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + Duration::days(i)
    }

    /// Closes starting at `start` and following the given log returns, one per day.
    fn series(start: f64, log_returns: &[f64]) -> Vec<DailyBar> {
        let mut close = start;
        let mut bars = vec![DailyBar {
            date: day(0),
            close,
        }];
        for (i, r) in log_returns.iter().enumerate() {
            close *= r.exp();
            bars.push(DailyBar {
                date: day(i64::try_from(i).unwrap() + 1),
                close,
            });
        }
        bars
    }

    fn loaded(bars: Vec<DailyBar>) -> LoadedHistory {
        LoadedHistory {
            bars,
            source: DataSource::Fallback {
                file: "TEST.csv".into(),
            },
        }
    }

    fn config(symbols: &[&str], window: usize, method: VolMethod) -> MarketConfig {
        MarketConfig {
            symbols: symbols.iter().map(|s| sym(s)).collect(),
            risk_free_rate: 0.04,
            dividend_yields: symbols.iter().map(|s| (sym(s), 0.01)).collect(),
            dividend_model: BTreeMap::new(),
            vol_window_days: window,
            vol_method: method,
            cache_max_age_hours: 24,
            data_mode: DataMode::Offline,
        }
    }

    fn single(bars: Vec<DailyBar>, window: usize, method: VolMethod) -> SeedData {
        let histories = BTreeMap::from([(sym("AAA"), loaded(bars))]);
        calibrate(&histories, &config(&["AAA"], window, method)).unwrap()
    }

    /// A wobbly but deterministic return pattern, no randomness.
    fn wobble(n: usize, scale: f64, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| scale * ((i as f64) * 0.9 + phase).sin() + 0.1 * scale * (i as f64).cos())
            .collect()
    }

    #[test]
    fn realised_vol_of_an_alternating_pattern() {
        // Returns +r, -r, +r, -r: mean 0, sample variance 4r²/3.
        let r = 0.01;
        let bars = series(100.0, &[r, -r, r, -r]);
        let seed = single(bars, 4, VolMethod::Realised);
        let expected = (4.0 * r * r / 3.0 * 252.0_f64).sqrt();
        assert_relative_eq!(
            seed.underlyings[&sym("AAA")].vol_realised,
            expected,
            epsilon = 1e-12
        );
        assert_relative_eq!(seed.underlyings[&sym("AAA")].vol, expected, epsilon = 1e-12);
    }

    #[test]
    fn realised_vol_uses_only_the_last_window_of_returns() {
        // A huge early return outside the window must not matter.
        let r = 0.01;
        let bars = series(100.0, &[0.5, r, -r, r, -r]);
        let seed = single(bars, 4, VolMethod::Realised);
        let expected = (4.0 * r * r / 3.0 * 252.0_f64).sqrt();
        assert_relative_eq!(
            seed.underlyings[&sym("AAA")].vol_realised,
            expected,
            epsilon = 1e-12
        );
    }

    #[test]
    fn ewma_variance_matches_a_hand_calculation() {
        // r = [0.01, -0.02, 0.03], lambda 0.94.
        // v0 = 1e-4, v1 = 0.94e-4 + 0.06 * 4e-4 = 1.18e-4,
        // v2 = 0.94 * 1.18e-4 + 0.06 * 9e-4 = 1.6492e-4.
        let v = ewma_variance(&[0.01, -0.02, 0.03]);
        assert_relative_eq!(v, 1.6492e-4, epsilon = 1e-15);
        assert_relative_eq!(
            ewma_vol(&[0.01, -0.02, 0.03]),
            (1.6492e-4_f64 * 252.0).sqrt(),
            epsilon = 1e-12
        );
    }

    #[test]
    fn vol_method_picks_which_vol_seeds() {
        let bars = series(100.0, &[0.01, -0.02, 0.03]);
        let realised = single(bars.clone(), 3, VolMethod::Realised);
        let ewma = single(bars, 3, VolMethod::Ewma);
        let r = &realised.underlyings[&sym("AAA")];
        let e = &ewma.underlyings[&sym("AAA")];
        assert_relative_eq!(r.vol, r.vol_realised);
        assert_relative_eq!(e.vol, e.vol_ewma);
        assert_relative_eq!(r.vol_ewma, e.vol_ewma);
        assert!((r.vol_realised - r.vol_ewma).abs() > 1e-3, "the two differ");
        assert_relative_eq!(r.spot, series(100.0, &[0.01, -0.02, 0.03])[3].close);
    }

    #[test]
    fn constant_returns_have_no_usable_vol() {
        let histories = BTreeMap::from([(sym("AAA"), loaded(series(100.0, &[0.01; 5])))]);
        let err = calibrate(&histories, &config(&["AAA"], 4, VolMethod::Realised)).unwrap_err();
        assert!(matches!(err, CalibrationError::BadVol { .. }), "{err:?}");
    }

    #[test]
    fn split_10_for_1_is_adjusted() {
        // 100 days around 1000 then a 10-for-1 split: the raw close drops to ~100.
        let mut returns = wobble(60, 0.01, 0.0);
        let mut bars = series(1000.0, &returns[..30]);
        let last_before = bars[bars.len() - 1].close;
        let move_on_split_day = 0.02_f64;
        let after = last_before / 10.0 * move_on_split_day.exp();
        let mut close = after;
        let split_date = day(31);
        bars.push(DailyBar {
            date: split_date,
            close,
        });
        for (i, r) in returns[31..].iter().enumerate() {
            close *= r.exp();
            bars.push(DailyBar {
                date: day(32 + i64::try_from(i).unwrap()),
                close,
            });
        }
        returns[30] = move_on_split_day;
        let (adjusted, splits) = adjust_splits(&sym("AAA"), &bars);
        assert_eq!(
            splits,
            [Split {
                date: split_date,
                ratio: 10.0
            }]
        );
        // Earlier closes are divided by 10, later ones untouched.
        assert_relative_eq!(adjusted[0].close, 100.0, epsilon = 1e-9);
        assert_relative_eq!(adjusted[31].close, bars[31].close);
        // The split-day return is the real 2 % move, not -90 %.
        let r = (adjusted[31].close / adjusted[30].close).ln();
        assert_relative_eq!(r, move_on_split_day, epsilon = 1e-12);
        // Vol with adjustment matches a series that never split; without it, it explodes.
        let unsplit: Vec<f64> = returns.iter().copied().take(bars.len() - 1).collect();
        let clean_vol = realised_vol(&unsplit[unsplit.len() - 50..]);
        let seed = single(bars.clone(), 50, VolMethod::Realised);
        let got = seed.underlyings[&sym("AAA")].vol_realised;
        assert!(got < 0.6, "adjusted vol {got}");
        let raw_returns = log_returns(&bars.iter().map(|b| b.close).collect::<Vec<_>>());
        let raw_vol = realised_vol(&raw_returns[raw_returns.len() - 50..]);
        assert!(raw_vol > 3.0 * got, "raw {raw_vol} vs adjusted {got}");
        assert!(clean_vol > 0.0);
    }

    #[test]
    fn split_detection_ratios_and_tolerance() {
        // Forward splits, including 3-for-2.
        for n in [1.5, 2.0, 3.0, 4.0, 5.0, 10.0, 20.0] {
            assert_eq!(detect_split(100.0 * n, 100.0), Some(n));
            // Just inside the 2 % window both ways.
            assert_eq!(detect_split(100.0 * n * 1.019, 100.0), Some(n));
            assert_eq!(detect_split(100.0 * n * 0.981, 100.0), Some(n));
            // Reverse splits.
            let inv = detect_split(100.0, 100.0 * n).unwrap();
            assert_relative_eq!(inv, 1.0 / n);
        }
        // Outside the window, and ordinary moves.
        assert_eq!(detect_split(100.0 * 10.0 * 1.03, 100.0), None);
        assert_eq!(detect_split(100.0, 100.0), None);
        assert_eq!(detect_split(108.0, 100.0), None);
        assert_eq!(detect_split(100.0, 108.0), None);
    }

    #[test]
    fn an_8_percent_move_is_left_alone() {
        let bars = series(100.0, &[0.02, -0.03, 0.0769, 0.01, -0.02]); // ~8 % up on day 3
        let (adjusted, splits) = adjust_splits(&sym("AAA"), &bars);
        assert!(splits.is_empty());
        assert_eq!(adjusted, bars);
    }

    #[test]
    fn a_reverse_split_and_a_3_for_2_split_are_adjusted() {
        let mut bars = series(10.0, &[0.01, 0.01, 0.01]);
        // 1-for-10 reverse split: price jumps tenfold.
        for b in &mut bars[2..] {
            b.close *= 10.0;
        }
        let (adjusted, splits) = adjust_splits(&sym("AAA"), &bars);
        assert_eq!(splits.len(), 1);
        assert_relative_eq!(splits[0].ratio, 0.1);
        assert_relative_eq!(adjusted[0].close, bars[0].close * 10.0, epsilon = 1e-9);

        let mut bars = series(150.0, &[0.01, 0.01, 0.01]);
        for b in &mut bars[2..] {
            b.close /= 1.5;
        }
        let (adjusted, splits) = adjust_splits(&sym("AAA"), &bars);
        assert_eq!(splits.len(), 1);
        assert_relative_eq!(splits[0].ratio, 1.5);
        assert_relative_eq!(adjusted[0].close, bars[0].close / 1.5, epsilon = 1e-9);
    }

    #[test]
    fn two_splits_compound() {
        let mut bars = series(1000.0, &[0.0, 0.0, 0.0, 0.0]);
        bars[2].close = 500.0; // 2-for-1 on day 2 (after two unchanged closes)
        bars[3].close = 500.0;
        bars[4].close = 50.0; // 10-for-1 on day 4
        let (adjusted, splits) = adjust_splits(&sym("AAA"), &bars);
        assert_eq!(splits.len(), 2);
        for b in &adjusted {
            assert_relative_eq!(b.close, 50.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn correlation_with_itself_and_its_negative() {
        let r = wobble(30, 0.01, 0.3);
        let neg: Vec<f64> = r.iter().map(|x| -x).collect();
        let other = wobble(30, 0.02, 1.7);
        let histories = BTreeMap::from([
            (sym("AAA"), loaded(series(100.0, &r))),
            (sym("BBB"), loaded(series(50.0, &r))),
            (sym("CCC"), loaded(series(80.0, &neg))),
            (sym("DDD"), loaded(series(20.0, &other))),
        ]);
        let cfg = config(&["AAA", "BBB", "CCC", "DDD"], 30, VolMethod::Realised);
        let seed = calibrate(&histories, &cfg).unwrap();
        let m = &seed.correlation;
        assert_relative_eq!(m[0][1], 1.0, epsilon = 1e-12);
        assert_relative_eq!(m[0][2], -1.0, epsilon = 1e-12);
        assert!(m[0][3].abs() < 1.0);
        for (i, row) in m.iter().enumerate() {
            assert_eq!(row[i].to_bits(), 1.0_f64.to_bits(), "exact unit diagonal");
            for (j, value) in row.iter().enumerate() {
                assert_eq!(value.to_bits(), m[j][i].to_bits(), "exact symmetry");
                assert!((-1.0..=1.0).contains(value));
            }
        }
        assert_eq!(seed.symbols.len(), 4);
        assert_eq!(seed.returns.len(), 30);
        assert_eq!(seed.returns[0].len(), 4);
        assert_eq!(seed.return_dates.len(), 30);
        assert_relative_eq!(seed.returns[5][0], r[r.len() - 30 + 5], epsilon = 1e-12);
    }

    #[test]
    fn correlation_aligns_by_date_with_an_inner_join() {
        // BBB is missing every third date, so only the shared dates line up.
        let r = wobble(60, 0.01, 0.3);
        let a = series(100.0, &r);
        let b: Vec<DailyBar> = a
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 1)
            .map(|(_, bar)| *bar)
            .collect();
        let histories = BTreeMap::from([(sym("AAA"), loaded(a)), (sym("BBB"), loaded(b))]);
        let cfg = config(&["AAA", "BBB"], 30, VolMethod::Realised);
        let seed = calibrate(&histories, &cfg).unwrap();
        assert_eq!(seed.returns.len(), 30);
        // Every return date is a date BBB has.
        assert!(
            seed.return_dates
                .iter()
                .all(|d| (*d - day(0)).num_days() % 3 != 1)
        );
        assert!(seed.correlation[0][1] > 0.5);
    }

    #[test]
    fn errors_are_typed() {
        let cfg = config(&["AAA", "BBB"], 4, VolMethod::Realised);
        let ok = loaded(series(100.0, &wobble(8, 0.01, 0.0)));
        let mut h = BTreeMap::new();
        assert_eq!(calibrate(&h, &cfg), Err(CalibrationError::NoHistories));
        h.insert(sym("AAA"), ok.clone());
        assert_eq!(
            calibrate(&h, &cfg),
            Err(CalibrationError::MissingHistory(sym("BBB")))
        );
        h.insert(sym("BBB"), loaded(series(100.0, &wobble(3, 0.01, 0.0))));
        assert!(matches!(
            calibrate(&h, &cfg),
            Err(CalibrationError::InsufficientHistory {
                have: 4,
                need: 5,
                ..
            })
        ));
        // Long enough each, but they share no dates.
        let mut late = series(100.0, &wobble(8, 0.01, 0.0));
        for b in &mut late {
            b.date += Duration::days(100);
        }
        h.insert(sym("BBB"), loaded(late));
        assert!(matches!(
            calibrate(&h, &cfg),
            Err(CalibrationError::InsufficientOverlap { need: 4, .. })
        ));
        // Bad closes and repeated dates.
        let mut bad = series(100.0, &wobble(8, 0.01, 0.0));
        bad[2].close = -1.0;
        h.insert(sym("BBB"), loaded(bad));
        assert!(matches!(
            calibrate(&h, &cfg),
            Err(CalibrationError::InvalidClose { .. })
        ));
        let mut dup = series(100.0, &wobble(8, 0.01, 0.0));
        dup[3].date = dup[2].date;
        h.insert(sym("BBB"), loaded(dup));
        assert!(matches!(
            calibrate(&h, &cfg),
            Err(CalibrationError::DuplicateDate { .. })
        ));
        // Missing dividend yield.
        let mut no_yield = cfg.clone();
        no_yield.dividend_yields.remove(&sym("AAA"));
        h.insert(sym("BBB"), ok);
        assert_eq!(
            calibrate(&h, &no_yield),
            Err(CalibrationError::MissingDividendYield(sym("AAA")))
        );
    }

    #[test]
    fn a_constant_series_among_others_is_zero_variance_or_bad_vol() {
        let cfg = config(&["AAA", "BBB"], 5, VolMethod::Realised);
        let flat = vec![
            DailyBar {
                date: day(0),
                close: 10.0
            };
            1
        ];
        let mut bars = flat;
        for i in 1..8 {
            bars.push(DailyBar {
                date: day(i),
                close: 10.0,
            });
        }
        let h = BTreeMap::from([
            (sym("AAA"), loaded(series(100.0, &wobble(7, 0.01, 0.0)))),
            (sym("BBB"), loaded(bars)),
        ]);
        assert!(calibrate(&h, &cfg).is_err());
    }

    #[test]
    fn seed_builds_a_market_state() {
        let histories = BTreeMap::from([
            (sym("AAA"), loaded(series(100.0, &wobble(8, 0.01, 0.0)))),
            (sym("BBB"), loaded(series(50.0, &wobble(8, 0.02, 1.0)))),
        ]);
        let cfg = config(&["AAA", "BBB"], 6, VolMethod::Realised);
        let seed = calibrate(&histories, &cfg).unwrap();
        assert_eq!(seed.as_of, day(8));
        assert_relative_eq!(seed.rate, 0.04);
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap();
        let market = seed.to_market_state(now).unwrap();
        assert_eq!(market.as_of(), now);
        assert_relative_eq!(market.rate(), 0.04);
        let a = market.underlying(&sym("AAA")).unwrap();
        assert_relative_eq!(a.spot(), seed.underlyings[&sym("AAA")].spot);
        assert_relative_eq!(a.vol(), seed.underlyings[&sym("AAA")].vol);
        assert_relative_eq!(a.dividend_yield(), 0.01);
        // A bad vol is a typed error, not a NaN.
        let mut broken = seed;
        if let Some(s) = broken.underlyings.get_mut(&sym("AAA")) {
            s.vol = 0.0;
        }
        assert!(matches!(
            broken.to_market_state(now),
            Err(CalibrationError::Domain(_))
        ));
    }

    #[test]
    fn seed_round_trips_through_json() {
        let histories =
            BTreeMap::from([(sym("AAA"), loaded(series(100.0, &wobble(8, 0.01, 0.0))))]);
        let seed = calibrate(&histories, &config(&["AAA"], 6, VolMethod::Ewma)).unwrap();
        let text = serde_json::to_string(&seed).unwrap();
        let back: SeedData = serde_json::from_str(&text).unwrap();
        // serde_json floats are accurate to the last digit or two, not always bit-exact.
        assert_eq!(back.as_of, seed.as_of);
        assert_eq!(back.symbols, seed.symbols);
        assert_eq!(back.return_dates, seed.return_dates);
        let (b, s0) = (
            &back.underlyings[&sym("AAA")],
            &seed.underlyings[&sym("AAA")],
        );
        assert_eq!(b.source, s0.source);
        assert_relative_eq!(b.spot, s0.spot, max_relative = 1e-12);
        assert_relative_eq!(b.vol, s0.vol, max_relative = 1e-12);
        assert_relative_eq!(back.returns[0][0], seed.returns[0][0], max_relative = 1e-12);
    }
}
