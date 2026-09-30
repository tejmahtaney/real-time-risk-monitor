//! The clock abstraction, so time can be fixed in tests and simulated later.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::error::DomainError;
use crate::sim_clock::ClockStep;

/// A source of the current time. [`SimClock`](crate::SimClock) is the simulated one.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> DateTime<Utc>;
}

/// A clock the engine can drive: it moves simulated time forward one step at a time and can
/// be paused and re-timed.
///
/// [`SimClock`](crate::SimClock) is the live one, moving by real time × speed through market
/// hours. [`ReplayClock`](crate::ReplayClock) is the backtester's, moving one trading date per
/// step. The engine is generic over this trait, so a backtest runs the same tick code as live
/// mode. `speed` means simulated seconds per real second for `SimClock` and dates per second
/// for `ReplayClock`.
pub trait EngineClock: Clock {
    /// Moves simulated time on and reports the step. `real_elapsed` is the real time since the
    /// previous call; a replay clock ignores it. A paused clock returns a zero-length step.
    fn advance(&mut self, real_elapsed: Duration) -> ClockStep;

    /// The speed multiplier.
    fn speed(&self) -> f64;

    /// Changes the speed for later steps.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidSpeed`] when `speed` is not finite and positive.
    fn set_speed(&mut self, speed: f64) -> Result<(), DomainError>;

    /// Stops time.
    fn pause(&mut self);

    /// Lets time move again.
    fn resume(&mut self);

    /// Whether the clock is paused.
    fn is_paused(&self) -> bool;
}

/// A clock that always returns the same instant. For tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(DateTime<Utc>);

impl FixedClock {
    /// A clock stuck at `instant`.
    pub fn new(instant: DateTime<Utc>) -> Self {
        Self(instant)
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn fixed_clock_does_not_move() {
        let t = Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
        let clock = FixedClock::new(t);
        assert_eq!(clock.now(), t);
        assert_eq!(clock.now(), t);
        let dynamic: &dyn Clock = &clock;
        assert_eq!(dynamic.now(), t);
    }
}
