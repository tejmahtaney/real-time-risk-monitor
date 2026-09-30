//! The local archive of every download: `market-data/{SYMBOL}/<UTC timestamp>-daily.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::Symbol;
use serde::{Deserialize, Serialize};
use storage::{StorageRoot, list, read_json, write_json_new};

use crate::error::MarketDataError;
use crate::source::{DailyBar, Download, RequestParams, parse_daily_body};

const FOLDER: &str = "market-data";
const STEM: &str = "daily";

/// One archived download. The API key is never part of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveRecord {
    /// The symbol downloaded.
    pub symbol: Symbol,
    /// When the download finished.
    pub fetched_at: DateTime<Utc>,
    /// The request parameters, without the API key.
    pub params: RequestParams,
    /// The response body exactly as received.
    pub body: String,
}

/// An archive file that was read and parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveHit {
    /// The file the record came from.
    pub path: PathBuf,
    /// The record.
    pub record: ArchiveRecord,
    /// The bars parsed from the record's body, oldest first.
    pub bars: Vec<DailyBar>,
}

/// Every archived download of one symbol merged into one series.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedHistory {
    /// One bar per date, oldest first. Where downloads overlap the newest one's close is used.
    pub bars: Vec<DailyBar>,
    /// Archive files merged.
    pub files_read: usize,
    /// Archive files skipped because they did not parse.
    pub files_skipped: usize,
    /// Dates on which two downloads disagreed by more than 1e-9 in relative terms.
    pub conflicts: usize,
}

/// The archive folder inside a storage root. Nothing in it is ever overwritten or deleted.
#[derive(Debug, Clone)]
pub struct Archive {
    root: StorageRoot,
}

impl Archive {
    /// An archive inside `root`. Folders are created on the first save.
    pub fn new(root: StorageRoot) -> Self {
        Self { root }
    }

    /// Saves a download as a new file and returns its path.
    ///
    /// # Errors
    ///
    /// [`MarketDataError::Storage`] when the folder or file cannot be written.
    pub fn save(
        &self,
        symbol: &Symbol,
        download: &Download,
        now: DateTime<Utc>,
    ) -> Result<PathBuf, MarketDataError> {
        let record = ArchiveRecord {
            symbol: symbol.clone(),
            fetched_at: now,
            params: download.params.clone(),
            body: download.raw_body.clone(),
        };
        let dir = self.root.subdir(&[FOLDER, symbol.as_str()])?;
        Ok(write_json_new(&dir, STEM, now, &record)?)
    }

    /// Every usable archive file for `symbol`, merged into one history without duplicate dates.
    ///
    /// Each free download holds only the latest 100 days, so the more often the app has fetched,
    /// the longer the merged history. Files are read oldest to newest, so where two downloads hold
    /// the same date the newer one wins; a disagreement is counted and logged, as it means the
    /// vendor restated a close. A file that does not parse is skipped with a warning.
    ///
    /// # Errors
    ///
    /// [`MarketDataError::Storage`] when the folder cannot be read or the path is unsafe.
    pub fn merged_history(&self, symbol: &Symbol) -> Result<MergedHistory, MarketDataError> {
        let dir = self.root.join(&[FOLDER, symbol.as_str()])?;
        let mut by_date: BTreeMap<NaiveDate, f64> = BTreeMap::new();
        let (mut files_read, mut files_skipped, mut conflicts) = (0, 0, 0);
        for path in list(&dir)? {
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bars = read_json::<ArchiveRecord>(&path)
                .map_err(|e| e.to_string())
                .and_then(|record| parse_daily_body(&record.body).map_err(|e| e.to_string()));
            let bars = match bars {
                Ok(bars) => bars,
                Err(reason) => {
                    tracing::warn!(file = %path.display(), %reason, "skipping unusable archive file");
                    files_skipped += 1;
                    continue;
                }
            };
            files_read += 1;
            for bar in bars {
                if let Some(old) = by_date.insert(bar.date, bar.close)
                    && (old - bar.close).abs() > 1e-9 * old.abs()
                {
                    conflicts += 1;
                    tracing::warn!(%symbol, date = %bar.date, old, new = bar.close, "archive downloads disagree on a close; the newer one is used");
                }
            }
        }
        Ok(MergedHistory {
            bars: by_date
                .into_iter()
                .map(|(date, close)| DailyBar { date, close })
                .collect(),
            files_read,
            files_skipped,
            conflicts,
        })
    }

    /// The newest usable archive file for `symbol`.
    ///
    /// With `max_age` it must have been fetched within that age of `now`, otherwise the
    /// answer is `None`; with `None` any age will do. A file that does not parse is
    /// skipped with a warning and the next newest is tried.
    ///
    /// # Errors
    ///
    /// [`MarketDataError::Storage`] when the folder cannot be read or the path is unsafe.
    pub fn newest(
        &self,
        symbol: &Symbol,
        now: DateTime<Utc>,
        max_age: Option<Duration>,
    ) -> Result<Option<ArchiveHit>, MarketDataError> {
        let dir = self.root.join(&[FOLDER, symbol.as_str()])?;
        for path in list(&dir)?.into_iter().rev() {
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let usable = read_json::<ArchiveRecord>(&path)
                .map_err(|e| e.to_string())
                .and_then(|record| {
                    let bars = parse_daily_body(&record.body).map_err(|e| e.to_string())?;
                    Ok((record, bars))
                });
            let (record, bars) = match usable {
                Ok(pair) => pair,
                Err(reason) => {
                    tracing::warn!(file = %path.display(), %reason, "skipping unusable archive file");
                    continue;
                }
            };
            if let Some(limit) = max_age
                && now - record.fetched_at > limit
            {
                // Files are in time order, so everything older is stale too.
                return Ok(None);
            }
            return Ok(Some(ArchiveHit { path, record, bars }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;
    use crate::source::sample_body;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, h, 0, 0).unwrap()
    }

    fn download(close: f64) -> Download {
        let raw_body = sample_body("SPY", &[("2026-09-25", close)]);
        Download {
            bars: parse_daily_body(&raw_body).unwrap(),
            raw_body,
            params: RequestParams {
                function: "TIME_SERIES_DAILY".into(),
                symbol: "SPY".into(),
                outputsize: "compact".into(),
                datatype: "json".into(),
            },
        }
    }

    fn archive() -> (tempfile::TempDir, Archive) {
        let dir = tempfile::tempdir().unwrap();
        let archive = Archive::new(StorageRoot::at(dir.path()));
        (dir, archive)
    }

    #[test]
    fn empty_archive_has_nothing() {
        let (_dir, archive) = archive();
        assert!(archive.newest(&sym("SPY"), at(12), None).unwrap().is_none());
    }

    #[test]
    fn two_saves_make_two_files_and_the_newest_wins() {
        let (dir, archive) = archive();
        let a = archive.save(&sym("SPY"), &download(1.0), at(9)).unwrap();
        let b = archive.save(&sym("SPY"), &download(2.0), at(9)).unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(dir.path().join("market-data").join("SPY")));
        assert!(a.to_string_lossy().ends_with("20260926T090000Z-daily.json"));
        let hit = archive.newest(&sym("SPY"), at(10), None).unwrap().unwrap();
        assert_eq!(hit.path, b);
        assert_relative_eq!(hit.bars[0].close, 2.0);
        assert_eq!(hit.record.fetched_at, at(9));
    }

    #[test]
    fn fresh_hit_stale_miss() {
        let (_dir, archive) = archive();
        archive.save(&sym("SPY"), &download(1.0), at(0)).unwrap();
        let day = Some(Duration::hours(24));
        assert!(archive.newest(&sym("SPY"), at(23), day).unwrap().is_some());
        let later = at(0) + Duration::hours(25);
        assert!(archive.newest(&sym("SPY"), later, day).unwrap().is_none());
        // Any age still finds it.
        assert!(archive.newest(&sym("SPY"), later, None).unwrap().is_some());
    }

    #[test]
    fn two_overlapping_downloads_merge_into_one_series_without_duplicate_dates() {
        let (dir, archive) = archive();
        let symbol = sym("SPY");
        let save = |bars: &[(&str, f64)], hour: u32| {
            let raw_body = sample_body("SPY", bars);
            let download = Download {
                bars: parse_daily_body(&raw_body).unwrap(),
                raw_body,
                params: RequestParams {
                    function: "TIME_SERIES_DAILY".into(),
                    symbol: "SPY".into(),
                    outputsize: "compact".into(),
                    datatype: "json".into(),
                },
            };
            archive.save(&symbol, &download, at(hour)).unwrap();
        };
        save(
            &[
                ("2026-09-21", 10.0),
                ("2026-09-22", 11.0),
                ("2026-09-23", 12.0),
            ],
            1,
        );
        // Overlaps on the 22nd and 23rd, restating the 23rd, and adds two later days.
        save(
            &[
                ("2026-09-22", 11.0),
                ("2026-09-23", 12.5),
                ("2026-09-24", 13.0),
                ("2026-09-25", 14.0),
            ],
            2,
        );
        std::fs::write(
            dir.path()
                .join("market-data")
                .join("SPY")
                .join("20260926T030000Z-daily.json"),
            "{ nope",
        )
        .unwrap();

        let merged = archive.merged_history(&symbol).unwrap();
        let dates: Vec<String> = merged.bars.iter().map(|b| b.date.to_string()).collect();
        assert_eq!(
            dates,
            [
                "2026-09-21",
                "2026-09-22",
                "2026-09-23",
                "2026-09-24",
                "2026-09-25"
            ]
        );
        // The newer download wins on the restated date.
        assert_relative_eq!(merged.bars[2].close, 12.5);
        assert_eq!(
            (merged.files_read, merged.files_skipped, merged.conflicts),
            (2, 1, 1)
        );
        // Nothing archived for another symbol.
        assert!(
            archive
                .merged_history(&sym("AAPL"))
                .unwrap()
                .bars
                .is_empty()
        );
    }

    #[test]
    fn symbols_do_not_mix() {
        let (_dir, archive) = archive();
        archive.save(&sym("SPY"), &download(1.0), at(0)).unwrap();
        assert!(archive.newest(&sym("AAPL"), at(1), None).unwrap().is_none());
    }

    #[test]
    fn a_corrupt_newest_file_falls_back_to_the_next() {
        let (dir, archive) = archive();
        archive.save(&sym("SPY"), &download(1.0), at(8)).unwrap();
        let folder = dir.path().join("market-data").join("SPY");
        std::fs::write(folder.join("20260926T090000Z-daily.json"), "{ nope").unwrap();
        let hit = archive.newest(&sym("SPY"), at(10), None).unwrap().unwrap();
        assert_relative_eq!(hit.bars[0].close, 1.0);
    }

    #[test]
    fn the_archive_holds_no_api_key_field() {
        let (_dir, archive) = archive();
        let path = archive.save(&sym("SPY"), &download(1.0), at(0)).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["body", "fetched_at", "params", "symbol"]);
        assert!(!text.to_ascii_lowercase().contains("apikey"));
    }
}
