//! Committed fallback price files: `data/fallback/{SYMBOL}.csv` with columns `date,close`.

use std::path::{Path, PathBuf};

use domain::Symbol;

use crate::error::MarketDataError;
use crate::source::DailyBar;

/// The fallback file for `symbol` inside `dir`.
pub fn fallback_path(dir: &Path, symbol: &Symbol) -> PathBuf {
    dir.join(format!("{symbol}.csv"))
}

fn fail(path: &Path, message: impl std::fmt::Display) -> MarketDataError {
    MarketDataError::Fallback {
        path: path.display().to_string(),
        message: message.to_string(),
    }
}

/// Reads the fallback file for `symbol`, oldest bar first, or `None` when there is none.
///
/// # Errors
///
/// [`MarketDataError::Fallback`] when the file exists but cannot be parsed, holds a close
/// that is not a positive finite number, repeats a date, or has no rows.
pub fn read_fallback(
    dir: &Path,
    symbol: &Symbol,
) -> Result<Option<Vec<DailyBar>>, MarketDataError> {
    let path = fallback_path(dir, symbol);
    if !path.is_file() {
        return Ok(None);
    }
    let mut reader = csv::Reader::from_path(&path).map_err(|e| fail(&path, e))?;
    let mut bars = Vec::new();
    for row in reader.deserialize::<DailyBar>() {
        let bar = row.map_err(|e| fail(&path, e))?;
        if !bar.close.is_finite() || bar.close <= 0.0 {
            return Err(fail(
                &path,
                format!(
                    "close {} on {} is not a positive number",
                    bar.close, bar.date
                ),
            ));
        }
        bars.push(bar);
    }
    bars.sort_by_key(|b| b.date);
    if bars.is_empty() {
        return Err(fail(&path, "no rows"));
    }
    let repeated = bars.windows(2).find_map(|w| match w {
        [a, b] if a.date == b.date => Some(a.date),
        _ => None,
    });
    if let Some(date) = repeated {
        return Err(fail(&path, format!("date {date} appears twice")));
    }
    Ok(Some(bars))
}

/// Writes the fallback file for `symbol`, oldest bar first, creating `dir` if needed.
///
/// This produces a committed seed file, not runtime data, so it is a plain write.
///
/// # Errors
///
/// [`MarketDataError::Fallback`] when the folder or file cannot be written.
pub fn write_fallback(
    dir: &Path,
    symbol: &Symbol,
    bars: &[DailyBar],
) -> Result<PathBuf, MarketDataError> {
    let path = fallback_path(dir, symbol);
    std::fs::create_dir_all(dir).map_err(|e| fail(&path, e))?;
    let mut sorted = bars.to_vec();
    sorted.sort_by_key(|b| b.date);
    let mut writer = csv::Writer::from_path(&path).map_err(|e| fail(&path, e))?;
    for bar in &sorted {
        writer.serialize(bar).map_err(|e| fail(&path, e))?;
    }
    writer.flush().map_err(|e| fail(&path, e))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn bar(d: &str, close: f64) -> DailyBar {
        DailyBar {
            date: NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(),
            close,
        }
    }

    #[test]
    fn round_trip_sorts_by_date() {
        let dir = tempfile::tempdir().unwrap();
        let bars = [bar("2026-09-25", 600.5), bar("2026-09-24", 598.25)];
        let path = write_fallback(dir.path(), &sym("SPY"), &bars).unwrap();
        assert_eq!(path, dir.path().join("SPY.csv"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "date,close\n2026-09-24,598.25\n2026-09-25,600.5\n");
        let back = read_fallback(dir.path(), &sym("SPY")).unwrap().unwrap();
        assert_eq!(back, [bar("2026-09-24", 598.25), bar("2026-09-25", 600.5)]);
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_fallback(dir.path(), &sym("SPY")).unwrap().is_none());
    }

    #[test]
    fn bad_files_are_typed_errors() {
        let dir = tempfile::tempdir().unwrap();
        for text in [
            "date,close\n",
            "date,close\n2026-09-24,abc\n",
            "date,close\nnot-a-date,1\n",
            "date,close\n2026-09-24,-1\n",
            "date,close\n2026-09-24,0\n",
            "date,close\n2026-09-24,1\n2026-09-24,2\n",
        ] {
            std::fs::write(dir.path().join("SPY.csv"), text).unwrap();
            assert!(
                matches!(
                    read_fallback(dir.path(), &sym("SPY")),
                    Err(MarketDataError::Fallback { .. })
                ),
                "{text:?}"
            );
        }
    }
}
