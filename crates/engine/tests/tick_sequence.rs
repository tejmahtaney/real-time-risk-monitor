//! Calling `Engine::tick` directly, with a fixed seed and no timers: `seq` increases
//! every call, the session rolls correctly across a close, and an option expiring
//! today settles at 16:00.

use std::time::Duration;

use chrono::NaiveDate;
use engine::PositionStatus;

mod support;

/// One real second at the fixture's 3,600x speed moves the clock by one simulated hour.
const ONE_SIM_HOUR: Duration = Duration::from_secs(1);

#[test]
fn seq_increases_by_one_every_tick() {
    let mut engine = support::build_engine();
    let mut previous_seq = 0;
    for _ in 0..12 {
        let snapshot = engine.tick(ONE_SIM_HOUR).unwrap();
        assert_eq!(snapshot.seq, previous_seq + 1);
        previous_seq = snapshot.seq;
    }
}

#[test]
fn session_rolls_and_the_zero_day_put_settles_at_the_close() {
    let mut engine = support::build_engine();
    let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    let tuesday = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();

    let mut rolled = false;
    let mut last_snapshot = None;
    for _ in 0..12 {
        let snapshot = engine.tick(ONE_SIM_HOUR).unwrap();
        if snapshot.session_date == tuesday {
            rolled = true;
        } else {
            assert_eq!(
                snapshot.session_date, monday,
                "session date should not drift"
            );
        }
        last_snapshot = Some(snapshot);
    }
    assert!(
        rolled,
        "the session should have rolled to Tuesday within 12 one-hour ticks"
    );

    let final_snapshot = last_snapshot.unwrap();
    let put = final_snapshot
        .positions
        .iter()
        .find(|p| p.id.as_str() == "spy-put-0d")
        .expect("spy-put-0d should still be in the book");
    assert_eq!(put.status, PositionStatus::Expired);

    // The attributor should have rolled the P&L day along with the session.
    assert!(final_snapshot.pnl.previous_day.is_some());
}
