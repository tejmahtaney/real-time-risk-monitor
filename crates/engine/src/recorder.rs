//! Session recording: batches ticks, events, closes and daily P&L into
//! `sessions/<id>/` under the storage root, flushing at least once a second and at
//! shutdown.
//!
//! # Why the engine never waits on this
//!
//! The tick-loop task hands records here with `mpsc::Sender::try_send` (capacity
//! [`RECORDER_CHANNEL_CAPACITY`]), exactly the back-pressure rule `server::ws` already
//! documents for its own `watch`/`broadcast` channels: a full channel is never blocked
//! on, it is just recognised (a record is counted as dropped) and moved past. A slow
//! disk, a burst of settlements, or this task falling behind can never delay a tick.
//! This mirrors `crates/server/src/ws.rs`'s `watch` channel, which likewise never
//! blocks its publisher; the two differ only in *what* is dropped (here, a counted
//! record; there, a superseded snapshot) and in using a bounded queue instead of "one
//! slot, always overwritten," because a session recording should keep as much history
//! as it reasonably can, unlike a live UI that only ever needs the newest value.
//!
//! # Where the disk is touched
//!
//! This module is the only place in this crate that performs any file I/O: not
//! [`crate::Engine::new`], not [`crate::Engine::apply`], not [`crate::Engine::tick`].
//! Those only ever build in-memory [`RecorderRecord`]s and hand them to [`run`] over a
//! channel. This keeps the engine's own tick and command paths free of anything that
//! could block on a slow disk.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{Position, PositionId};
use portfolio::SettlementEvent;
use risk::{AlertEvent, PnlBuckets};
use serde::{Deserialize, Serialize};
use storage::StorageRoot;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::snapshot::{HistoryPoint, RiskSnapshot};

/// Capacity of the engine-to-recorder channel. The tick loop's `try_send` never waits
/// on this filling; see this module's doc comment.
pub(crate) const RECORDER_CHANNEL_CAPACITY: usize = 10_000;

/// `sessions/<id>/meta.json`. Written at session start with `end: None`; rewritten in
/// place at shutdown with `end` set and the final `dropped_records`. A session whose
/// `end` is `None` was cut short (crash, `kill -9`) — readers must treat that as "still
/// running," never as a parse error.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct SessionMeta {
    /// This session's id: a UTC start time plus a short random suffix.
    pub session_id: String,
    /// When the session started.
    pub start: DateTime<Utc>,
    /// When the session ended, or `None` if it is still running or was cut short.
    pub end: Option<DateTime<Utc>>,
    /// `config/simulation.toml`'s RNG seed. Not the calibrated market seed.
    pub seed: u64,
    /// The clock's speed multiplier at session start.
    pub speed: f64,
    /// The git commit this session's code was built from, or `"unknown"`.
    pub git_commit: String,
    /// Records dropped because the recorder's channel was full, over the whole
    /// session. Only accurate once `end` is set; `0` while the session is still open.
    pub dropped_records: u64,
}

/// Whether a position was added to or removed from the book.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PositionChangeKind {
    /// The position was added.
    Added,
    /// The position was removed.
    Removed,
}

/// A pause, resume or speed change applied to the engine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ControlAction {
    /// Simulated time stopped advancing.
    Pause,
    /// Simulated time resumed advancing.
    Resume,
    /// The clock's speed multiplier changed.
    SetSpeed {
        /// The new speed.
        speed: f64,
    },
}

/// One entry in `sessions/<id>/events.jsonl`: an alert, a settlement, a position
/// change, a portfolio load, or a control action. Tagged by `type`, so `GET
/// /api/sessions/{id}/events?type=` can filter on the wire shape directly.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SessionEvent {
    /// A limit's state changed.
    Alert(AlertEvent),
    /// An option settled at expiry.
    Settlement(SettlementEvent),
    /// A position was added to or removed from the book.
    PositionChange {
        /// Real-world time of the change.
        wall_time: DateTime<Utc>,
        /// Simulated time of the change.
        sim_time: DateTime<Utc>,
        /// Added or removed.
        kind: PositionChangeKind,
        /// The affected position's id.
        position_id: PositionId,
    },
    /// The whole book was replaced by a loaded portfolio version.
    PortfolioLoad {
        /// Real-world time of the load.
        wall_time: DateTime<Utc>,
        /// Simulated time of the load.
        sim_time: DateTime<Utc>,
        /// Where the loaded positions came from — in practice, a version filename.
        source: String,
        /// How many positions the book holds after the load.
        position_count: usize,
    },
    /// Pause, resume or a speed change.
    Control {
        /// Real-world time of the action.
        wall_time: DateTime<Utc>,
        /// Simulated time of the action.
        sim_time: DateTime<Utc>,
        /// What was done.
        action: ControlAction,
    },
}

/// One row of `sessions/<id>/daily-pnl.csv`: every P&L bucket for one simulated day.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct DailyPnlRow {
    /// The New York date this row covers.
    pub session_date: NaiveDate,
    /// Δ·dS·scale, summed over the day.
    pub delta: f64,
    /// ½·Γ·dS²·scale, summed over the day.
    pub gamma: f64,
    /// ν·dσ·scale, summed over the day.
    pub vega: f64,
    /// Θ·dt·scale, summed over the day.
    pub theta: f64,
    /// ρ·dr·scale, summed over the day.
    pub rho: f64,
    /// The cross and pure-vol-convexity terms, summed over the day.
    pub higher_order: f64,
    /// `actual` minus the six modelled buckets above, summed over the day.
    pub residual: f64,
    /// The day's true change in value, from full revaluation.
    pub actual: f64,
}

impl DailyPnlRow {
    fn new(session_date: NaiveDate, buckets: PnlBuckets) -> Self {
        Self {
            session_date,
            delta: buckets.delta,
            gamma: buckets.gamma,
            vega: buckets.vega,
            theta: buckets.theta,
            rho: buckets.rho,
            higher_order: buckets.higher_order,
            residual: buckets.residual,
            actual: buckets.actual,
        }
    }
}

/// One thing the tick loop hands to the recorder. Stays crate-private: `server` only
/// ever sees the file-format types above ([`SessionEvent`], [`DailyPnlRow`],
/// [`SessionMeta`]), never this envelope.
#[derive(Debug, Clone)]
pub(crate) enum RecorderRecord {
    /// One tick's history point, appended to `history.jsonl`.
    History(HistoryPoint),
    /// One event, appended to `events.jsonl`.
    Event(SessionEvent),
    /// A full snapshot at a session close, written to `closes/<date>.json`.
    Close {
        session_date: NaiveDate,
        snapshot: Arc<RiskSnapshot>,
    },
    /// One day's final P&L buckets, appended as a row to `daily-pnl.csv`.
    DailyPnl {
        session_date: NaiveDate,
        buckets: PnlBuckets,
    },
    /// The book's active (non-settled) positions, rewritten to `portfolios/current.json`.
    CurrentPortfolio(Vec<Position>),
}

/// Runs until `records` closes (every `Sender` dropped, which the tick loop does once
/// it exits its own `select!` on shutdown).
///
/// First writes the initial `meta.json` (`end: None`). Then, every second, and once
/// more after the channel closes, flushes whatever `history`/`events` lines have
/// accumulated; `Close`/`DailyPnl`/`CurrentPortfolio` records are written immediately,
/// since they are rare (at most once a simulated day, or once per add/remove/load) and
/// do not need batching. On close, drains anything left with `try_recv`, flushes once
/// more, then rewrites `meta.json` with `end` set — the required "drain, flush, write
/// final meta.json" shutdown sequence.
///
/// All I/O errors here are logged and otherwise swallowed: a recording failure must
/// never propagate back and disturb the engine.
pub(crate) async fn run(
    mut records: mpsc::Receiver<RecorderRecord>,
    storage: StorageRoot,
    mut meta: SessionMeta,
    dropped_records: Arc<AtomicU64>,
) {
    let session_dir = match storage.subdir(&["sessions", &meta.session_id]) {
        Ok(dir) => dir,
        Err(err) => {
            tracing::error!(error = %err, "cannot create the session folder; recording is disabled for this session");
            return;
        }
    };
    if let Err(err) = storage::write_json_atomic(&session_dir.join("meta.json"), &meta) {
        tracing::error!(error = %err, "failed to write the initial session meta.json");
    }

    let mut history_batch: Vec<HistoryPoint> = Vec::new();
    let mut events_batch: Vec<SessionEvent> = Vec::new();
    let mut flush_interval = tokio::time::interval(Duration::from_secs(1));
    flush_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            record = records.recv() => {
                match record {
                    Some(r) => apply_record(r, &storage, &session_dir, &mut history_batch, &mut events_batch),
                    None => break,
                }
            }
            _ = flush_interval.tick() => flush(&session_dir, &mut history_batch, &mut events_batch),
        }
    }
    while let Ok(r) = records.try_recv() {
        apply_record(
            r,
            &storage,
            &session_dir,
            &mut history_batch,
            &mut events_batch,
        );
    }
    flush(&session_dir, &mut history_batch, &mut events_batch);

    meta.end = Some(Utc::now());
    meta.dropped_records = dropped_records.load(Ordering::Relaxed);
    if let Err(err) = storage::write_json_atomic(&session_dir.join("meta.json"), &meta) {
        tracing::error!(error = %err, "failed to write the final session meta.json");
    }
}

fn apply_record(
    record: RecorderRecord,
    storage: &StorageRoot,
    session_dir: &Path,
    history_batch: &mut Vec<HistoryPoint>,
    events_batch: &mut Vec<SessionEvent>,
) {
    match record {
        RecorderRecord::History(point) => history_batch.push(point),
        RecorderRecord::Event(event) => events_batch.push(event),
        RecorderRecord::Close {
            session_date,
            snapshot,
        } => {
            let dir = session_dir.join("closes");
            match std::fs::create_dir_all(&dir) {
                Ok(()) => {
                    let path = dir.join(format!("{session_date}.json"));
                    if let Err(err) = storage::write_json_atomic(&path, &snapshot) {
                        tracing::error!(error = %err, "failed to write a session close snapshot");
                    }
                }
                Err(err) => tracing::error!(error = %err, "cannot create the closes/ folder"),
            }
        }
        RecorderRecord::DailyPnl {
            session_date,
            buckets,
        } => {
            if let Err(err) =
                append_daily_pnl_row(session_dir, DailyPnlRow::new(session_date, buckets))
            {
                tracing::error!(error = %err, "failed to append a daily-pnl.csv row");
            }
        }
        RecorderRecord::CurrentPortfolio(positions) => match storage.subdir(&["portfolios"]) {
            Ok(dir) => {
                let path = dir.join("current.json");
                if let Err(err) = storage::write_json_atomic(&path, &positions) {
                    tracing::error!(error = %err, "failed to rewrite portfolios/current.json");
                }
            }
            Err(err) => tracing::error!(error = %err, "cannot reach portfolios/"),
        },
    }
}

fn flush(
    session_dir: &Path,
    history_batch: &mut Vec<HistoryPoint>,
    events_batch: &mut Vec<SessionEvent>,
) {
    if !history_batch.is_empty() {
        if let Err(err) = storage::append_jsonl(&session_dir.join("history.jsonl"), history_batch) {
            tracing::error!(error = %err, "failed to append to history.jsonl");
        }
        history_batch.clear();
    }
    if !events_batch.is_empty() {
        if let Err(err) = storage::append_jsonl(&session_dir.join("events.jsonl"), events_batch) {
            tracing::error!(error = %err, "failed to append to events.jsonl");
        }
        events_batch.clear();
    }
}

/// Appends one row to `session_dir/daily-pnl.csv`, writing the header first if the file
/// is new.
fn append_daily_pnl_row(session_dir: &Path, row: DailyPnlRow) -> Result<(), std::io::Error> {
    let path = session_dir.join("daily-pnl.csv");
    let is_new = !path.exists();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(file);
    if is_new {
        wtr.write_record([
            "session_date",
            "delta",
            "gamma",
            "vega",
            "theta",
            "rho",
            "higher_order",
            "residual",
            "actual",
        ])
        .map_err(std::io::Error::other)?;
    }
    wtr.serialize(row).map_err(std::io::Error::other)?;
    wtr.flush()
}

/// Copies each of `paths` into `sessions/<session_id>/config/`, so a session's folder
/// is self-contained. Missing source files are skipped with a warning rather than
/// failing the whole copy, since a session should still record even if one config path
/// is slightly wrong.
///
/// # Errors
///
/// [`storage::StorageError`] when the `config/` folder itself cannot be created.
pub fn copy_session_config(
    storage: &StorageRoot,
    session_id: &str,
    paths: &[&Path],
) -> Result<(), storage::StorageError> {
    let dir = storage.subdir(&["sessions", session_id, "config"])?;
    for path in paths {
        let Some(name) = path.file_name() else {
            continue;
        };
        if let Err(err) = std::fs::copy(path, dir.join(name)) {
            tracing::warn!(path = %path.display(), error = %err, "could not copy a config file into the session folder");
        }
    }
    Ok(())
}

/// `<UTC timestamp>-<4 random base-36 chars>`, e.g. `20260926T041500Z-k3f9`. Sorts by
/// time since the timestamp prefix does. Pure apart from reading the wall clock and
/// process id to seed a local RNG — no I/O.
pub(crate) fn new_session_id(start: DateTime<Utc>) -> String {
    format!("{}-{}", storage::timestamp_name(start), random_suffix())
}

/// Characters a session id's random suffix is drawn from.
const SUFFIX_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn random_suffix() -> String {
    use rand::{RngExt, SeedableRng};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seed = now.as_secs() ^ u64::from(now.subsec_nanos()) ^ u64::from(std::process::id());
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    (0..4)
        .map(|_| SUFFIX_ALPHABET[rng.random_range(0..SUFFIX_ALPHABET.len())] as char)
        .collect()
}

/// Compiled-in commit hash if `git` was available at build time
/// ([`crate`]'s `build.rs`), else `RRM_GIT_COMMIT` (Docker sets this in Part 17, since
/// a container image typically has no `.git` folder), else `"unknown"`.
pub(crate) fn git_commit() -> String {
    option_env!("RRM_GIT_COMMIT_BUILD_TIME")
        .map(str::to_owned)
        .or_else(|| std::env::var("RRM_GIT_COMMIT").ok())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn t() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, 4, 15, 0).unwrap()
    }

    #[test]
    fn session_id_is_timestamp_plus_four_char_suffix() {
        let id = new_session_id(t());
        assert!(id.starts_with("20260926T041500Z-"));
        let suffix = id.strip_prefix("20260926T041500Z-").unwrap();
        assert_eq!(suffix.len(), 4);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn git_commit_never_panics_and_is_never_empty() {
        // `unsafe_code` is forbidden workspace-wide, so this does not mutate the
        // environment; it only checks that whatever source wins (the build-time hash,
        // `RRM_GIT_COMMIT`, or the "unknown" fallback) produces a sensible value.
        assert!(!git_commit().is_empty());
    }

    #[test]
    fn a_full_channel_drops_records_without_the_sender_waiting() {
        let (tx, rx) = mpsc::channel::<RecorderRecord>(4);
        drop(rx); // never read, but try_send still must not block or panic
        for i in 0..8 {
            let result = tx.try_send(RecorderRecord::Event(SessionEvent::Control {
                wall_time: t(),
                sim_time: t(),
                action: ControlAction::SetSpeed {
                    speed: f64::from(i),
                },
            }));
            assert!(
                result.is_err(),
                "a closed/full channel must report Err, not block"
            );
        }
    }

    #[tokio::test]
    async fn run_writes_initial_and_final_meta_json_with_dropped_count() {
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageRoot::at(dir.path());
        let (tx, rx) = mpsc::channel(4);
        let dropped = Arc::new(AtomicU64::new(0));
        let meta = SessionMeta {
            session_id: "20260926T041500Z-test".to_owned(),
            start: t(),
            end: None,
            seed: 42,
            speed: 3_600.0,
            git_commit: "abc123".to_owned(),
            dropped_records: 0,
        };
        let handle = tokio::spawn(run(rx, storage.clone(), meta, dropped.clone()));

        // Give the task a moment to write the initial meta.json.
        tokio::task::yield_now().await;
        let path = dir
            .path()
            .join("sessions")
            .join("20260926T041500Z-test")
            .join("meta.json");
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let initial: SessionMeta = storage::read_json(&path).unwrap();
        assert!(initial.end.is_none());

        dropped.store(3, Ordering::Relaxed);
        drop(tx);
        handle.await.unwrap();

        let final_meta: SessionMeta = storage::read_json(&path).unwrap();
        assert!(final_meta.end.is_some());
        assert_eq!(final_meta.dropped_records, 3);
    }
}
