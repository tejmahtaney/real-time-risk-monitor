//! An idle subscriber never slows the engine down: a `watch::Receiver` that nobody
//! reads doesn't queue anything, and doesn't degrade tick performance over time. This
//! is the engine-level half of Part 12's WebSocket streaming claim ("the engine never
//! waits for a client and no queue grows without bound"); `server`'s own
//! `tests/ws.rs` covers the complementary end-to-end claim over a real socket.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use tokio_util::sync::CancellationToken;

mod support;

/// The middle value of `samples`, which does not need to be sorted beforehand.
fn median(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

#[tokio::test(start_paused = true)]
async fn an_unread_snapshot_receiver_does_not_slow_the_engine() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(10);
    let handle = engine.spawn(tick_interval, CancellationToken::new());

    // Clone a receiver and never read from it again: exactly what an idle WebSocket
    // client's task would hold onto. The engine's publish side does not know or care
    // that this receiver exists, let alone that nobody is polling it.
    let _idle_receiver = handle.subscribe_snapshots();

    let mut compute_micros = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        tokio::time::advance(tick_interval).await;
        tokio::task::yield_now().await;
        compute_micros.push(handle.latest().compute_micros);
    }

    assert_eq!(
        handle.latest().seq,
        1_000,
        "every tick should have advanced seq by one, idle receiver or not"
    );

    // Compare medians rather than requiring "no growth at all": a strict comparison
    // is brittle against ordinary scheduling jitter, but real unbounded growth (a
    // leak, or an O(n) cost creeping in per subscriber) would blow well past a 3x
    // margin over 1,000 ticks.
    let first = median(&compute_micros[..100]);
    let last = median(&compute_micros[compute_micros.len() - 100..]);
    assert!(
        last < first.max(1) * 3,
        "tick time should not grow over 1,000 ticks with an idle subscriber: \
         first-100 median {first}us, last-100 median {last}us"
    );
}
