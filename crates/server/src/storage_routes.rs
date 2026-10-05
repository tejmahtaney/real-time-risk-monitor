//! Storage-facing REST endpoints: usage, sessions, portfolio versions, and the
//! market-data archive listing. Kept separate from `routes.rs` to stop that module
//! growing unwieldy; merged into the same router before `.with_state` is applied there.

use std::collections::BTreeMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::atomic::Ordering;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, NaiveDateTime, Utc};
use engine::{
    DailyPnlRow, EngineCommand, EngineReply, HistoryPoint, RiskSnapshot, SessionEvent, SessionMeta,
    StorageRoot,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;
use crate::validate::safe_segment;

/// The storage-facing routes, merged into the main router before `.with_state` is
/// applied there.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/storage", get(storage_usage))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(session_detail))
        .route("/api/sessions/{id}/history", get(session_history))
        .route("/api/sessions/{id}/events", get(session_events))
        .route(
            "/api/portfolio/versions",
            get(list_versions).post(save_version),
        )
        .route("/api/portfolio/versions/{file}", get(get_version))
        .route(
            "/api/portfolio/versions/{file}/specs",
            get(get_version_specs),
        )
        .route("/api/portfolio/versions/{file}/load", post(load_version))
        .route("/api/market-data/archive", get(market_data_archive))
        .route(
            "/api/market-data/archive/{symbol}/{file}/closes",
            get(archive_file_closes),
        )
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct StorageUsageResponse {
    usage_bytes: BTreeMap<String, u64>,
    total_bytes: u64,
    limit_bytes: u64,
    warn_fraction: f64,
    warning: bool,
    storage_path: String,
}

async fn storage_usage(
    State(state): State<AppState>,
) -> Result<Json<StorageUsageResponse>, ApiError> {
    let usage_bytes = engine::usage_by_folder(&state.storage_root)?;
    let total_bytes = usage_bytes.values().sum();
    Ok(Json(StorageUsageResponse {
        usage_bytes,
        total_bytes,
        limit_bytes: state.storage_config.max_bytes,
        warn_fraction: state.storage_config.warn_fraction,
        warning: state.storage_warning.load(Ordering::Relaxed),
        storage_path: state.storage_root.path().display().to_string(),
    }))
}

/// Reads `sessions/<id>/meta.json`, both to hand back to the caller and to turn an
/// unknown session id into a 404 (via `StorageError::Io`'s not-found mapping) before
/// any of the other per-session endpoints read anything else.
fn load_session_meta(
    storage_root: &StorageRoot,
    id: &str,
) -> Result<(PathBuf, SessionMeta), ApiError> {
    let dir = storage_root.join(&["sessions", id])?;
    let meta: SessionMeta = engine::read_json(&dir.join("meta.json"))?;
    Ok((dir, meta))
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct SessionSummary {
    id: String,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    simulated_days: usize,
    size_bytes: u64,
    git_commit: String,
    total_pnl: Option<f64>,
}

async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionSummary>>, ApiError> {
    let sessions_dir = state.storage_root.join(&["sessions"])?;
    let mut out = Vec::new();
    for dir in engine::list_dirs(&sessions_dir)? {
        let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let meta: SessionMeta = engine::read_json(&dir.join("meta.json"))?;
        let closes = engine::list(&dir.join("closes"))?;
        let total_pnl = closes
            .last()
            .map(|path| engine::read_json::<RiskSnapshot>(path))
            .transpose()?
            .map(|snapshot| snapshot.pnl.portfolio.since_start.actual);
        out.push(SessionSummary {
            id: id.to_owned(),
            start: meta.start,
            end: meta.end,
            simulated_days: closes.len(),
            size_bytes: engine::size_of(&dir)?,
            git_commit: meta.git_commit,
            total_pnl,
        });
    }
    Ok(Json(out))
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct SessionDetail {
    meta: SessionMeta,
    daily_pnl: Vec<DailyPnlRow>,
}

async fn session_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionDetail>, ApiError> {
    safe_segment(&id)?;
    let (dir, meta) = load_session_meta(&state.storage_root, &id)?;
    let daily_pnl = read_daily_pnl_csv(&dir.join("daily-pnl.csv"));
    Ok(Json(SessionDetail { meta, daily_pnl }))
}

/// A missing `daily-pnl.csv` (no session close yet) reads as an empty list, matching
/// every other storage reader's "missing means nothing recorded yet" rule.
fn read_daily_pnl_csv(path: &StdPath) -> Vec<DailyPnlRow> {
    let Ok(mut reader) = csv::Reader::from_path(path) else {
        return Vec::new();
    };
    reader
        .deserialize::<DailyPnlRow>()
        .filter_map(Result::ok)
        .collect()
}

#[derive(Debug, Deserialize)]
struct HistoryRangeQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    max_points: Option<usize>,
}

async fn session_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HistoryRangeQuery>,
) -> Result<Json<Vec<HistoryPoint>>, ApiError> {
    safe_segment(&id)?;
    let (dir, _meta) = load_session_meta(&state.storage_root, &id)?;
    let mut points: Vec<HistoryPoint> = engine::read_jsonl(&dir.join("history.jsonl"))?;
    if let Some(from) = query.from {
        points.retain(|p| p.sim_time >= from);
    }
    if let Some(to) = query.to {
        points.retain(|p| p.sim_time <= to);
    }
    Ok(Json(downsample(points, query.max_points.unwrap_or(2000))))
}

/// Keeps at most `max_points` points from `points`, bucketing by index into
/// `ceil(max_points / 2)` buckets and keeping the point with the minimum and the point
/// with the maximum `dollar_delta` in each bucket (deduplicated when they are the same
/// point), then re-sorted by `seq`. `dollar_delta` — the portfolio's flagship risk
/// series — is the bucketing key so a returned point always stays a real,
/// internally-consistent tick (every other field on it is exactly what that tick
/// measured), rather than downsampling every field independently, which would misalign
/// correlated series on a chart. A no-op when `points.len() <= max_points` or
/// `max_points` is `0`.
fn downsample(points: Vec<HistoryPoint>, max_points: usize) -> Vec<HistoryPoint> {
    if max_points == 0 || points.len() <= max_points {
        return points;
    }
    let bucket_count = (max_points / 2).max(1);
    let bucket_size = points.len().div_ceil(bucket_count);
    let mut out = Vec::with_capacity(max_points);
    for chunk in points.chunks(bucket_size) {
        let Some(first) = chunk.first() else {
            continue;
        };
        let mut min = first.clone();
        let mut max = first.clone();
        for point in chunk {
            if point.dollar_delta < min.dollar_delta {
                min = point.clone();
            }
            if point.dollar_delta > max.dollar_delta {
                max = point.clone();
            }
        }
        out.push(min.clone());
        if max.seq != min.seq {
            out.push(max);
        }
    }
    out.sort_by_key(|point| point.seq);
    out
}

#[derive(Debug, Deserialize)]
struct EventTypeQuery {
    r#type: Option<String>,
}

async fn session_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventTypeQuery>,
) -> Result<Json<Vec<SessionEvent>>, ApiError> {
    safe_segment(&id)?;
    let (dir, _meta) = load_session_meta(&state.storage_root, &id)?;
    let events: Vec<SessionEvent> = engine::read_jsonl(&dir.join("events.jsonl"))?;
    let events = match query.r#type {
        Some(wanted) => events
            .into_iter()
            .filter(|event| event_type_tag(event) == wanted)
            .collect(),
        None => events,
    };
    Ok(Json(events))
}

/// Must match `SessionEvent`'s `#[serde(tag = "type", rename_all = "snake_case")]`
/// wire tag exactly, so `?type=` filters on the same string a client reads off each
/// event's own `"type"` field.
fn event_type_tag(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::Alert(_) => "alert",
        SessionEvent::Settlement(_) => "settlement",
        SessionEvent::PositionChange { .. } => "position_change",
        SessionEvent::PortfolioLoad { .. } => "portfolio_load",
        SessionEvent::CorporateAction(_) => "corporate_action",
        SessionEvent::Control { .. } => "control",
    }
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct VersionSummary {
    file: String,
    size_bytes: u64,
    /// When the version was saved, from the UTC stamp in its file name.
    saved_at: DateTime<Utc>,
    /// The name it was saved under, as words (the file name carries the slug).
    name: String,
    /// True for the automatic `session-start` version, false for one saved by name.
    auto: bool,
}

/// The stem of the version the server saves by itself at each start.
const AUTO_VERSION_STEM: &str = "session-start";

/// Splits `<UTC stamp>-<stem>[_NN].csv` (see `storage::write_bytes_new`) into its time, display
/// name and auto flag. `None` for a file not named that way.
fn parse_version_name(file: &str) -> Option<(DateTime<Utc>, String, bool)> {
    let base = file.strip_suffix(".csv")?;
    let (stamp, rest) = base.split_once('-')?;
    let saved_at = NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
        .ok()?
        .and_utc();
    // A name clash within one second appends `_NN`; it is not part of the name.
    let stem = match rest.rsplit_once('_') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => rest,
    };
    Some((saved_at, stem.replace('-', " "), stem == AUTO_VERSION_STEM))
}

async fn list_versions(
    State(state): State<AppState>,
) -> Result<Json<Vec<VersionSummary>>, ApiError> {
    let dir = state.storage_root.join(&["portfolios", "versions"])?;
    let mut out = Vec::new();
    for path in engine::list(&dir)? {
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((saved_at, name, auto)) = parse_version_name(file) else {
            continue;
        };
        out.push(VersionSummary {
            file: file.to_owned(),
            size_bytes: engine::size_of(&path)?,
            saved_at,
            name,
            auto,
        });
    }
    Ok(Json(out))
}

#[derive(Deserialize, ts_rs::TS)]
#[ts(export)]
struct SaveVersionRequest {
    name: String,
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct SavedVersionResponse {
    file: String,
}

/// Saving a version is the handler's job, not the engine's: it asks the engine for the
/// current active specs (`EngineCommand::GetSpecs`, which touches no disk) and then
/// writes the CSV itself, keeping every file write off the engine's tick-loop task.
async fn save_version(
    State(state): State<AppState>,
    Json(request): Json<SaveVersionRequest>,
) -> Result<Json<SavedVersionResponse>, ApiError> {
    let EngineReply::Specs(specs) = state.engine.send(EngineCommand::GetSpecs).await?? else {
        return Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        ));
    };
    let slug = engine::slugify(&request.name);
    let bytes = engine::to_csv_bytes(&specs)?;
    let dir = state.storage_root.subdir(&["portfolios", "versions"])?;
    let path = engine::write_bytes_new(&dir, &slug, "csv", Utc::now(), &bytes)?;
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&slug)
        .to_owned();
    Ok(Json(SavedVersionResponse { file }))
}

async fn get_version(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<(HeaderMap, Vec<u8>), ApiError> {
    safe_segment(&file)?;
    let path = state
        .storage_root
        .join(&["portfolios", "versions", &file])?;
    let bytes = std::fs::read(&path).map_err(|source| engine::StorageError::Io {
        action: "read",
        path: path.clone(),
        source,
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv"));
    Ok((headers, bytes))
}

/// `GET /api/portfolio/versions/{file}/specs`: the version's positions as JSON, so a client can
/// compare it with the book without parsing CSV.
async fn get_version_specs(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<Json<Vec<engine::PositionSpec>>, ApiError> {
    safe_segment(&file)?;
    let path = state
        .storage_root
        .join(&["portfolios", "versions", &file])?;
    let bytes = std::fs::read(&path).map_err(|source| engine::StorageError::Io {
        action: "read",
        path: path.clone(),
        source,
    })?;
    Ok(Json(engine::parse_csv(bytes.as_slice())?))
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct LoadedVersionResponse {
    position_count: usize,
}

/// Loading a version is likewise the handler's job: it reads and parses the CSV itself,
/// then hands the engine already-resolved specs via `EngineCommand::LoadPortfolio`, so
/// the engine task never opens a file.
async fn load_version(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<Json<LoadedVersionResponse>, ApiError> {
    safe_segment(&file)?;
    let path = state
        .storage_root
        .join(&["portfolios", "versions", &file])?;
    let bytes = std::fs::read(&path).map_err(|source| engine::StorageError::Io {
        action: "read",
        path: path.clone(),
        source,
    })?;
    let specs = engine::parse_csv(bytes.as_slice())?;
    match state
        .engine
        .send(EngineCommand::LoadPortfolio {
            specs,
            source: file,
        })
        .await??
    {
        EngineReply::Loaded(position_count) => Ok(Json(LoadedVersionResponse { position_count })),
        _ => Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        )),
    }
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct ArchiveFileInfo {
    file: String,
    fetched_at: DateTime<Utc>,
    size_bytes: u64,
    /// Where the download came from: the vendor and its function. `unreadable` if the file
    /// does not parse.
    source: String,
    /// How many dates the download holds. `0` if the file does not parse.
    dates: usize,
}

/// One date and close of an archived download.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct ArchiveClose {
    date: chrono::NaiveDate,
    close: f64,
}

fn read_archive_file(path: &StdPath) -> Option<(String, Vec<engine::DailyBar>)> {
    let record: engine::ArchiveRecord = engine::read_json(path).ok()?;
    let bars = engine::parse_daily_body(&record.body).ok()?;
    Some((format!("Alpha Vantage {}", record.params.function), bars))
}

/// `GET /api/market-data/archive/{symbol}/{file}/closes`: the closes of one archived download,
/// oldest first. Kept off the list so that stays light.
async fn archive_file_closes(
    State(state): State<AppState>,
    Path((symbol, file)): Path<(String, String)>,
) -> Result<Json<Vec<ArchiveClose>>, ApiError> {
    safe_segment(&symbol)?;
    safe_segment(&file)?;
    let path = state.storage_root.join(&["market-data", &symbol, &file])?;
    // Missing file is a 404 through the storage error's not-found mapping.
    std::fs::metadata(&path).map_err(|source| engine::StorageError::Io {
        action: "read",
        path: path.clone(),
        source,
    })?;
    let Some((_, bars)) = read_archive_file(&path) else {
        return Err(ApiError::Validation(format!(
            "{file} is not a readable download"
        )));
    };
    Ok(Json(
        bars.into_iter()
            .map(|bar| ArchiveClose {
                date: bar.date,
                close: bar.close,
            })
            .collect(),
    ))
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct SymbolArchive {
    symbol: String,
    files: Vec<ArchiveFileInfo>,
}

/// The fetch time is parsed straight out of each file's name (the `storage::timestamp_name`
/// prefix). The source and date count need each file opened, but the closes stay behind their
/// own route.
async fn market_data_archive(
    State(state): State<AppState>,
) -> Result<Json<Vec<SymbolArchive>>, ApiError> {
    let root = state.storage_root.join(&["market-data"])?;
    let mut out = Vec::new();
    for dir in engine::list_dirs(&root)? {
        let Some(symbol) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut files = Vec::new();
        for path in engine::list(&dir)? {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(prefix) = name.split('-').next() else {
                continue;
            };
            let Ok(naive) = NaiveDateTime::parse_from_str(prefix, "%Y%m%dT%H%M%SZ") else {
                continue;
            };
            let (source, dates) = match read_archive_file(&path) {
                Some((source, bars)) => (source, bars.len()),
                None => ("unreadable".to_owned(), 0),
            };
            files.push(ArchiveFileInfo {
                file: name.to_owned(),
                fetched_at: naive.and_utc(),
                size_bytes: engine::size_of(&path)?,
                source,
                dates,
            });
        }
        out.push(SymbolArchive {
            symbol: symbol.to_owned(),
            files,
        });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use engine::PnlBuckets;

    use super::*;

    fn point(seq: u64, dollar_delta: f64) -> HistoryPoint {
        HistoryPoint {
            seq,
            sim_time: Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap(),
            dollar_delta,
            dollar_gamma_1pct: 0.0,
            vega: 0.0,
            theta: 0.0,
            gross_exposure: 0.0,
            market_value: 0.0,
            pnl_today: PnlBuckets::default(),
            worst_limit_utilisation: 0.0,
            var_99: None,
        }
    }

    #[test]
    fn version_names_split_into_time_name_and_auto_flag() {
        let (at, name, auto) = parse_version_name("20260928T120227Z-session-start.csv").unwrap();
        assert_eq!(at, Utc.with_ymd_and_hms(2026, 9, 28, 12, 2, 27).unwrap());
        assert_eq!(name, "session start");
        assert!(auto);

        let (_, name, auto) =
            parse_version_name("20260928T120320Z-live-smoke-test_02.csv").unwrap();
        assert_eq!(name, "live smoke test");
        assert!(!auto);

        assert!(parse_version_name("notes.csv").is_none());
        assert!(parse_version_name("badstamp-x.csv").is_none());
    }

    #[test]
    fn downsample_is_a_no_op_below_the_limit() {
        let points: Vec<_> = (0..5_u64).map(|i| point(i, i as f64)).collect();
        assert_eq!(downsample(points.clone(), 10), points);
        assert_eq!(downsample(points.clone(), 0), points);
    }

    #[test]
    fn downsample_keeps_the_global_min_and_max_and_stays_within_the_limit() {
        // A spike at seq 37 and a trough at seq 63, both well away from either end.
        let mut points: Vec<_> = (0..100).map(|i| point(i, 100.0)).collect();
        points[37].dollar_delta = 10_000.0;
        points[63].dollar_delta = -10_000.0;

        let out = downsample(points, 20);
        assert!(out.len() <= 20, "got {} points", out.len());
        assert!(out.windows(2).all(|w| w[0].seq < w[1].seq));
        assert!(
            out.iter()
                .any(|p| (p.dollar_delta - 10_000.0).abs() < f64::EPSILON)
        );
        assert!(
            out.iter()
                .any(|p| (p.dollar_delta - (-10_000.0)).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn event_type_tags_match_the_wire_shape() {
        let when = Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap();
        let control = SessionEvent::Control {
            wall_time: when,
            sim_time: when,
            action: engine::ControlAction::Pause,
        };
        let corporate = SessionEvent::CorporateAction(engine::CorporateActionEvent {
            id: "split".to_owned(),
            wall_time: when,
            sim_time: when,
            action: engine::CorporateAction::split(
                engine::Symbol::new("AAPL").unwrap(),
                4.0,
                when.date_naive(),
            )
            .unwrap(),
            outcome: engine::CorporateActionOutcome::Applied,
            message: "AAPL 4-for-1 split: 2 positions adjusted".to_owned(),
            positions_adjusted: 2,
            cash_paid: 0.0,
        });
        for event in [control, corporate] {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["type"], event_type_tag(&event));
        }
    }
}
