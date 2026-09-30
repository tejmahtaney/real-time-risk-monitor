//! Reading finished and unfinished runs back from `backtests/<run_id>/`.
//!
//! Results of a saved run come from its files alone, so a run survives a restart and needs no replay.

use std::path::{Path, PathBuf};

use risk::{BacktestSummary, DailyRow};
use storage::StorageRoot;

use super::error::BacktestError;
use super::types::{
    BACKTESTS_FOLDER, BacktestEvent, BacktestSettings, DAILY_FILE, EVENTS_FILE, SETTINGS_FILE,
    SUMMARY_FILE,
};

/// A run as read from disk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SavedRun {
    pub settings: BacktestSettings,
    pub rows: Vec<DailyRow>,
    pub events: Vec<BacktestEvent>,
    /// `None` for a run that never finished.
    pub summary: Option<BacktestSummary>,
}

/// The folder of run `id`, if `id` is a safe name.
pub(crate) fn run_dir(storage: &StorageRoot, id: &str) -> Result<PathBuf, BacktestError> {
    storage::check_part(id)?;
    Ok(storage.join(&[BACKTESTS_FOLDER, id])?)
}

/// Reads a whole run.
///
/// # Errors
///
/// [`BacktestError::NotFound`] when there is no such run, [`BacktestError::Storage`] for a file
/// that cannot be read.
pub(crate) fn read_saved(storage: &StorageRoot, id: &str) -> Result<SavedRun, BacktestError> {
    let dir = run_dir(storage, id)?;
    let settings = read_settings(&dir)
        .map_err(|_| BacktestError::NotFound(format!("no backtest run {id}")))?;
    Ok(SavedRun {
        settings,
        rows: read_rows(&dir)?,
        events: storage::read_jsonl(&dir.join(EVENTS_FILE))?,
        summary: read_summary(&dir)?,
    })
}

/// `settings.json` of the run in `dir`.
pub(crate) fn read_settings(dir: &Path) -> Result<BacktestSettings, storage::StorageError> {
    storage::read_json(&dir.join(SETTINGS_FILE))
}

/// `summary.json` of the run in `dir`, or `None` when the run is unfinished.
pub(crate) fn read_summary(dir: &Path) -> Result<Option<BacktestSummary>, storage::StorageError> {
    let path = dir.join(SUMMARY_FILE);
    if !path.exists() {
        return Ok(None);
    }
    storage::read_json(&path).map(Some)
}

/// The rows of `daily.csv`. A missing or empty file is no rows. A row that does not parse ends
/// the list with a warning, since only the last line of an append-only file can be half written.
pub(crate) fn read_rows(dir: &Path) -> Result<Vec<DailyRow>, BacktestError> {
    let path = dir.join(DAILY_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut reader = csv::Reader::from_path(&path)
        .map_err(|e| BacktestError::Internal(format!("cannot read {}: {e}", path.display())))?;
    let mut rows = Vec::new();
    for record in reader.deserialize::<DailyRow>() {
        match record {
            Ok(row) => rows.push(row),
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "ignoring an unreadable trailing row in daily.csv");
                break;
            }
        }
    }
    Ok(rows)
}

/// The id of every run folder, newest first (ids start with a UTC timestamp).
pub(crate) fn run_ids(storage: &StorageRoot) -> Result<Vec<String>, BacktestError> {
    let base = storage.path().join(BACKTESTS_FOLDER);
    let mut ids: Vec<String> = storage::list_dirs(&base)?
        .iter()
        .filter_map(|dir| dir.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .collect();
    ids.sort();
    ids.reverse();
    Ok(ids)
}
