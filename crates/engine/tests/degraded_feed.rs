//! A feed failure means stale prices, not stopped time: `Engine::tick` marks the
//! snapshot degraded but keeps running settlement, attribution and limits against
//! whatever the market last held.

use std::sync::atomic::Ordering;
use std::time::Duration;

use engine::FeedStatus;

mod support;

const ONE_SIM_HOUR: Duration = Duration::from_secs(1);

#[test]
fn a_failed_tick_freezes_the_market_but_not_the_risk_pipeline() {
    let (mut engine, fail) = support::build_failing_engine();

    let good = engine.tick(ONE_SIM_HOUR).unwrap();
    assert_eq!(good.feed_status, FeedStatus::Ok);

    fail.store(true, Ordering::SeqCst);
    let degraded = engine.tick(ONE_SIM_HOUR).unwrap();
    assert_eq!(degraded.feed_status, FeedStatus::Degraded);

    // The market genuinely did not move: same spot and vol per symbol as the last good
    // tick.
    assert_eq!(degraded.market, good.market);

    // But time still passed, so the risk pipeline still ran against those stale
    // prices: seq, sim time and the P&L attribution's "today" theta all moved on.
    assert_eq!(degraded.seq, good.seq + 1);
    assert!(degraded.sim_time > good.sim_time);
    for id in ["spy-put-0d", "aapl-call-30d"] {
        let before = good.positions.iter().find(|p| p.id.as_str() == id).unwrap();
        let after = degraded
            .positions
            .iter()
            .find(|p| p.id.as_str() == id)
            .unwrap();
        assert!(
            after.time_to_expiry < before.time_to_expiry,
            "{id}'s time to expiry should still decay on a degraded tick"
        );
    }

    // Recovery: once the feed is healthy again, prices move and the status clears.
    fail.store(false, Ordering::SeqCst);
    let recovered = engine.tick(ONE_SIM_HOUR).unwrap();
    assert_eq!(recovered.feed_status, FeedStatus::Ok);
    assert_ne!(recovered.market, degraded.market);
}

#[test]
fn settlement_and_the_overnight_day_roll_still_happen_on_a_degraded_tick() {
    let (mut engine, fail) = support::build_failing_engine();

    // Fail the feed for the whole session, including the close and the overnight jump.
    fail.store(true, Ordering::SeqCst);
    let mut rolled = false;
    let mut last = None;
    for _ in 0..12 {
        let snapshot = engine.tick(ONE_SIM_HOUR).unwrap();
        assert_eq!(snapshot.feed_status, FeedStatus::Degraded);
        if snapshot.pnl.previous_day.is_some() {
            rolled = true;
        }
        last = Some(snapshot);
    }
    assert!(
        rolled,
        "the P&L day should roll even while the feed stays degraded"
    );

    let last = last.unwrap();
    let put = last
        .positions
        .iter()
        .find(|p| p.id.as_str() == "spy-put-0d")
        .expect("spy-put-0d should still be in the book");
    assert_eq!(
        put.status,
        engine::PositionStatus::Expired,
        "settlement runs off sim time, not feed health"
    );
}
