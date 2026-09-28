//! Driving a spawned engine through `EngineHandle::send`: pause/resume/speed and
//! add/remove positions.

use std::time::Duration;

use engine::{EngineCommand, EngineCommandError, EngineReply, InstrumentKind, PortfolioError};
use tokio_util::sync::CancellationToken;

mod support;

/// Advances the paused Tokio clock and yields enough times for the engine's spawned
/// loop to observe it and tick.
async fn let_ticks_happen(count: u32, tick_interval: Duration) {
    for _ in 0..count {
        tokio::time::advance(tick_interval).await;
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn pause_stops_sim_time_and_resume_restarts_it() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(50);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(tick_interval, storage, CancellationToken::new());

    let ack = handle.send(EngineCommand::Pause).await.unwrap().unwrap();
    assert!(matches!(ack, EngineReply::Ack));

    let baseline = handle.latest().sim_time;
    let_ticks_happen(10, tick_interval).await;
    assert_eq!(
        handle.latest().sim_time,
        baseline,
        "sim time must not move while paused"
    );

    let ack = handle.send(EngineCommand::Resume).await.unwrap().unwrap();
    assert!(matches!(ack, EngineReply::Ack));
    let_ticks_happen(10, tick_interval).await;
    assert!(
        handle.latest().sim_time > baseline,
        "sim time should move again after resume"
    );
}

#[tokio::test(start_paused = true)]
async fn set_speed_changes_the_snapshot_and_bad_speeds_are_rejected() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(50);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(tick_interval, storage, CancellationToken::new());

    let ack = handle
        .send(EngineCommand::SetSpeed(240.0))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(ack, EngineReply::Ack));
    let_ticks_happen(1, tick_interval).await;
    approx::assert_relative_eq!(handle.latest().clock_speed, 240.0);

    for bad in [0.5, 5_000.0] {
        let result = handle.send(EngineCommand::SetSpeed(bad)).await.unwrap();
        assert!(
            matches!(result, Err(EngineCommandError::InvalidSpeed(v)) if (v - bad).abs() < f64::EPSILON),
            "speed {bad} should be rejected, got {result:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn add_and_remove_positions() {
    let engine = support::build_engine();
    let tick_interval = Duration::from_millis(50);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(tick_interval, storage, CancellationToken::new());

    let new_spec = engine::PositionSpec {
        id: engine::PositionId::new("aapl-stock").unwrap(),
        underlying: engine::Symbol::new("AAPL").unwrap(),
        kind: InstrumentKind::Equity,
        strike: None,
        expiry: None,
        quantity: 5.0,
        entry_price: None,
        iv: None,
    };

    let reply = handle
        .send(EngineCommand::AddPosition(new_spec.clone()))
        .await
        .unwrap()
        .unwrap();
    let EngineReply::Added(row) = reply else {
        unreachable!("expected Added, got {reply:?}");
    };
    assert_eq!(row.id.as_str(), "aapl-stock");
    let_ticks_happen(1, tick_interval).await;
    assert!(
        handle
            .latest()
            .positions
            .iter()
            .any(|p| p.id.as_str() == "aapl-stock")
    );

    // A duplicate id is rejected.
    let result = handle
        .send(EngineCommand::AddPosition(new_spec))
        .await
        .unwrap();
    assert!(
        matches!(
            result,
            Err(EngineCommandError::Portfolio(PortfolioError::DuplicateId(
                _
            )))
        ),
        "duplicate id should be rejected, got {result:?}"
    );

    // Removing a known id works, and it disappears from the next snapshot.
    let known_id = engine::PositionId::new("spy-stock").unwrap();
    let reply = handle
        .send(EngineCommand::RemovePosition(known_id))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reply, EngineReply::Removed(_)));
    let_ticks_happen(1, tick_interval).await;
    assert!(
        !handle
            .latest()
            .positions
            .iter()
            .any(|p| p.id.as_str() == "spy-stock")
    );

    // Removing an unknown id is rejected.
    let unknown_id = engine::PositionId::new("does-not-exist").unwrap();
    let result = handle
        .send(EngineCommand::RemovePosition(unknown_id))
        .await
        .unwrap();
    assert!(
        matches!(
            result,
            Err(EngineCommandError::Portfolio(
                PortfolioError::UnknownPosition(_)
            ))
        ),
        "unknown id should be rejected, got {result:?}"
    );
}
