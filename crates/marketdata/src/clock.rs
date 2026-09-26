//! The wall clock.

use chrono::{DateTime, Utc};
use domain::Clock;

/// A [`Clock`] that reads the system time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
