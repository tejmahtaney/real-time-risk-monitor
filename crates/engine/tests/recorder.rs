//! Session recording end to end: a spawned engine, ticked through a session close,
//! writes files that parse back, drains and flushes cleanly on shutdown, and never
//! lets a settled position come back to life in `portfolios/current.json`.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use chrono::NaiveDate;
use engine::{HistoryPoint, RiskSnapshot, SessionEvent, SessionMeta};
use tokio_util::sync::CancellationToken;

mod support;

/// One real second at the fixture's 3,600x speed moves the clock by one simulated
/// hour, matching `tick_sequence.rs`'s own convention.
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Advances the paused Tokio clock `count` times and yields so the spawned engine
/// task observes each one.
async fn advance(count: u32) {
    for _ in 0..count {
        tokio::time::advance(TICK_INTERVAL).await;
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_session_crossing_the_close_records_everything_and_shuts_down_cleanly() {
    let engine = support::build_engine();
    let session_id = engine.session_id().to_owned();
    let (dir, storage) = support::temp_storage();
    let shutdown = CancellationToken::new();
    let (handle, join) = engine.spawn(TICK_INTERVAL, storage.clone(), shutdown.clone());

    // 09:30 to 16:00 is 6.5 simulated hours; 10 one-hour advances comfortably cross
    // the close and the overnight roll, the same margin `tick_sequence.rs` uses.
    advance(10).await;
    let final_seq = handle.latest().seq;

    shutdown.cancel();
    join.await.unwrap();

    let session_dir = storage.join(&["sessions", &session_id]).unwrap();

    let history: Vec<HistoryPoint> =
        engine::read_jsonl(&session_dir.join("history.jsonl")).unwrap();
    assert_eq!(
        u64::try_from(history.len()).unwrap(),
        final_seq,
        "one history line per Engine::tick call"
    );
    assert!(history.windows(2).all(|w| w[0].seq < w[1].seq));

    let events: Vec<SessionEvent> = engine::read_jsonl(&session_dir.join("events.jsonl")).unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Settlement(_))),
        "the zero-day put should have settled and been recorded, got {events:?}"
    );

    let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    let close_path = session_dir.join("closes").join(format!("{monday}.json"));
    let close_snapshot: RiskSnapshot = engine::read_json(&close_path).unwrap();
    assert_eq!(close_snapshot.session_date, monday);

    let daily_pnl_path = session_dir.join("daily-pnl.csv");
    let mut reader = csv::Reader::from_path(&daily_pnl_path).unwrap();
    let rows: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 1, "one row for Monday's close");
    assert_eq!(rows[0].get(0), Some("2026-09-28"));

    let meta: SessionMeta = engine::read_json(&session_dir.join("meta.json")).unwrap();
    assert!(meta.end.is_some(), "shutdown should set the end time");
    assert!(meta.end.unwrap() >= meta.start);
    assert_eq!(meta.dropped_records, 0);

    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn a_settled_position_is_absent_from_current_json() {
    let engine = support::build_engine();
    let (dir, storage) = support::temp_storage();
    let shutdown = CancellationToken::new();
    let (_handle, join) = engine.spawn(TICK_INTERVAL, storage.clone(), shutdown.clone());

    // Cross the close, where the zero-day put settles.
    advance(10).await;
    shutdown.cancel();
    join.await.unwrap();

    let current: Vec<domain::Position> =
        engine::read_json(&storage.join(&["portfolios", "current.json"]).unwrap()).unwrap();
    assert!(
        !current.iter().any(|p| p.id.as_str() == "spy-put-0d"),
        "a settled option must not be in current.json: {current:?}"
    );
    // The still-active positions must still be there.
    assert!(current.iter().any(|p| p.id.as_str() == "spy-stock"));

    drop(dir);
}
