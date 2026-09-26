//! The clock abstraction, so time can be fixed in tests and simulated later.

use chrono::{DateTime, Utc};

/// A source of the current time. The simulated clock arrives in Part 8.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> DateTime<Utc>;
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
