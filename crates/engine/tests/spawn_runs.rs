//! Running a spawned engine for about a simulated second on a paused Tokio clock: it
//! produces several distinct snapshots, and stops cleanly on cancellation.

use std::time::Duration;

use engine::{EngineCommand, EngineError};
use tokio_util::sync::CancellationToken;

mod support;

#[tokio::test(start_paused = true)]
async fn spawned_engine_produces_several_snapshots_and_stops_on_cancel() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(100);
    let shutdown = CancellationToken::new();
    let (_dir, storage) = support::temp_storage();
    let (handle, join) = engine.spawn(tick_interval, storage, shutdown.clone());

    let mut seen = std::collections::BTreeSet::new();
    seen.insert(handle.latest().seq);
    for _ in 0..10 {
        tokio::time::advance(tick_interval).await;
        tokio::task::yield_now().await;
        seen.insert(handle.latest().seq);
    }
    assert!(
        seen.len() > 1,
        "several distinct snapshots should have been observed, got {seen:?}"
    );

    shutdown.cancel();
    // The tick loop's JoinHandle resolves only once it has also drained and awaited
    // the recorder task, so awaiting it directly (no timer involved, so this works
    // fine under this test's paused Tokio clock) confirms the whole shutdown sequence
    // completes.
    join.await.expect("the engine task should not panic");

    let result = handle.send(EngineCommand::Pause).await;
    assert!(
        matches!(result, Err(EngineError::Gone)),
        "the engine task should have stopped, got {result:?}"
    );
}
