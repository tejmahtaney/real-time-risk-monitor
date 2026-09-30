//! The backtest recorder: the only place a backtest run touches the disk.
//!
//! It follows the pattern of the session recorder ([`crate::recorder`]): the replay job sends
//! records over a channel and this task does every file write, so disk speed can never slow the
//! replay. It differs in one way. A session recorder may drop records when its bounded channel is
//! full, because a live tick must never wait. A backtest's `daily.csv` **is** its result, so
//! losing a row would be wrong. The channel here is therefore unbounded: the replay sends one
//! small row per date, so it cannot grow past the length of the run, and `send` never blocks.
//!
//! A run is finished when `summary.json` exists, so that file is written last, after `daily.csv`
//! and `events.jsonl` are complete.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use risk::{BacktestSummary, DailyRow};
use storage::StorageRoot;
use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;

use super::types::{
    BACKTESTS_FOLDER, BacktestEvent, BacktestSettings, DAILY_FILE, EVENTS_FILE, INPUT_FILE,
    SETTINGS_FILE, SUMMARY_FILE,
};
use crate::recorder::copy_config_files;

/// One thing the replay job hands to the recorder.
pub(crate) enum BacktestRecord {
    /// Creates the run's folder and writes `input.csv`, `settings.json` and the config copies.
    /// The recorder answers on `ack` once they are on disk.
    Init {
        input: Vec<u8>,
        settings: Box<BacktestSettings>,
        config_paths: Vec<PathBuf>,
        ack: oneshot::Sender<Result<(), String>>,
    },
    /// One date's row, appended to `daily.csv`.
    Daily(DailyRow),
    /// One event, appended to `events.jsonl`.
    Event(BacktestEvent),
    /// The final summary, written to `summary.json` after everything else.
    Summary(BacktestSummary),
}

/// Runs until every sender is dropped, then flushes and returns.
pub(crate) async fn run(
    mut records: mpsc::UnboundedReceiver<BacktestRecord>,
    storage: StorageRoot,
    run_id: String,
) {
    let dir = match storage.subdir(&[BACKTESTS_FOLDER, &run_id]) {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!(%error, "cannot create the backtest folder; the run will not be saved");
            while let Some(record) = records.recv().await {
                if let BacktestRecord::Init { ack, .. } = record {
                    let _ = ack.send(Err(error.to_string()));
                }
            }
            return;
        }
    };
    let mut writer = DailyWriter::new(&dir);
    let mut events: Vec<BacktestEvent> = Vec::new();
    let mut flush_interval = tokio::time::interval(Duration::from_secs(1));
    flush_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            record = records.recv() => match record {
                Some(record) => apply(record, &storage, &dir, &mut writer, &mut events),
                None => break,
            },
            _ = flush_interval.tick() => flush_events(&dir, &mut events),
        }
    }
    flush_events(&dir, &mut events);
}

fn apply(
    record: BacktestRecord,
    storage: &StorageRoot,
    dir: &Path,
    writer: &mut DailyWriter,
    events: &mut Vec<BacktestEvent>,
) {
    match record {
        BacktestRecord::Init {
            input,
            settings,
            config_paths,
            ack,
        } => {
            let result = init(storage, dir, &input, &settings, &config_paths);
            if let Err(error) = &result {
                tracing::error!(%error, "cannot write the backtest's starting files");
            }
            let _ = ack.send(result);
        }
        BacktestRecord::Daily(row) => writer.append(&row),
        BacktestRecord::Event(event) => events.push(event),
        BacktestRecord::Summary(summary) => {
            flush_events(dir, events);
            if let Err(error) = storage::write_json_atomic(&dir.join(SUMMARY_FILE), &summary) {
                tracing::error!(%error, "failed to write the backtest's summary.json");
            }
        }
    }
}

fn init(
    storage: &StorageRoot,
    dir: &Path,
    input: &[u8],
    settings: &BacktestSettings,
    config_paths: &[PathBuf],
) -> Result<(), String> {
    std::fs::write(dir.join(INPUT_FILE), input).map_err(|e| e.to_string())?;
    let paths: Vec<&Path> = config_paths.iter().map(PathBuf::as_path).collect();
    copy_config_files(
        storage,
        &[BACKTESTS_FOLDER, &settings.run_id, "config"],
        &paths,
    )
    .map_err(|e| e.to_string())?;
    storage::write_json_atomic(&dir.join(SETTINGS_FILE), settings).map_err(|e| e.to_string())
}

fn flush_events(dir: &Path, events: &mut Vec<BacktestEvent>) {
    if events.is_empty() {
        return;
    }
    if let Err(error) = storage::append_jsonl(&dir.join(EVENTS_FILE), events) {
        tracing::error!(%error, "failed to append to the backtest's events.jsonl");
    }
    events.clear();
}

/// Appends rows to `daily.csv`, keeping the file open. The header is written with the first row.
struct DailyWriter {
    path: PathBuf,
    writer: Option<csv::Writer<File>>,
}

impl DailyWriter {
    fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(DAILY_FILE),
            writer: None,
        }
    }

    fn append(&mut self, row: &DailyRow) {
        if self.writer.is_none() {
            match File::create(&self.path) {
                Ok(file) => self.writer = Some(csv::Writer::from_writer(file)),
                Err(error) => {
                    tracing::error!(%error, "cannot create the backtest's daily.csv");
                    return;
                }
            }
        }
        if let Some(writer) = self.writer.as_mut() {
            let written = writer
                .serialize(row)
                .and_then(|()| writer.flush().map_err(Into::into));
            if let Err(error) = written {
                tracing::error!(%error, "failed to append a row to the backtest's daily.csv");
            }
        }
    }
}
