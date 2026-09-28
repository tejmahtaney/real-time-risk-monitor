//! History buffer: filtering by `since`, and the ring buffer never exceeding its
//! configured capacity.
//!
//! `unwrap_used` is allowed for this whole file for the same reason `tests/support`
//! itself allows it: `build_engine_with_history_capacity` is a plain helper function,
//! not a `#[test]`-annotated one, so clippy's `allow-unwrap-in-tests` config does not
//! cover it even though this file only ever compiles as a test binary.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use engine::{Engine, EngineCommand, EngineReply, FixedClock, GbmSimulator, SimClock};
use tokio_util::sync::CancellationToken;

mod support;

/// Builds a fixture engine directly (not through `support::build_engine`, which
/// hardcodes its own capacity) with a small `history_capacity` so eviction is easy to
/// observe within a handful of ticks.
fn build_engine_with_history_capacity(capacity: usize) -> Engine<GbmSimulator> {
    let seed = support::seed_data();
    let config = support::sim_config();
    let feed = GbmSimulator::new(&seed, &config).unwrap();
    let clock = SimClock::new(support::start(), config.speed).unwrap();
    Engine::new(
        clock,
        &seed,
        feed,
        &support::positions(),
        support::limits_config(),
        Box::new(FixedClock::new(support::start())),
        capacity,
        support::RNG_SEED,
    )
    .unwrap()
}

#[test]
fn history_never_exceeds_capacity_and_keeps_the_most_recent_points() {
    let mut engine = build_engine_with_history_capacity(5);
    for _ in 0..10 {
        engine.tick(Duration::from_millis(250)).unwrap();
    }
    let points = engine.history_since(None);
    assert_eq!(
        points.len(),
        5,
        "the ring buffer should be capped at capacity"
    );
    // `Engine::new` publishes tick 0 (seq 0), then each of the 10 `tick()` calls
    // increments `seq` by one, so the most recent 5 points are seq 6..=10.
    let seqs: Vec<u64> = points.iter().map(|p| p.seq).collect();
    assert_eq!(seqs, vec![6, 7, 8, 9, 10]);
}

#[test]
fn history_since_excludes_points_at_or_before_the_given_seq() {
    let mut engine = build_engine_with_history_capacity(20);
    for _ in 0..10 {
        engine.tick(Duration::from_millis(250)).unwrap();
    }

    let points = engine.history_since(Some(7));
    assert!(points.iter().all(|p| p.seq > 7));
    assert_eq!(points.len(), 3, "seq 8, 9 and 10 should remain");

    let all = engine.history_since(None);
    assert_eq!(all.len(), 11, "tick 0 plus 10 ticks");
}

#[tokio::test(start_paused = true)]
async fn get_history_command_returns_points_through_a_spawned_handle() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(50);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(tick_interval, storage, CancellationToken::new());

    for _ in 0..5 {
        tokio::time::advance(tick_interval).await;
        tokio::task::yield_now().await;
    }

    let reply = handle
        .send(EngineCommand::GetHistory { since: None })
        .await
        .unwrap()
        .unwrap();
    let EngineReply::History(points) = reply else {
        unreachable!("expected History, got {reply:?}");
    };
    assert!(
        points.len() >= 2,
        "at least a couple of ticks should have happened"
    );
    assert!(
        points.windows(2).all(|w| w[0].seq < w[1].seq),
        "points should be in ascending seq order"
    );
}
