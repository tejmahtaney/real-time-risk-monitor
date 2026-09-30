//! The backtester's clock: one step per trading date, each ending at 16:00 New York time.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};

use crate::clock::{Clock, EngineClock};
use crate::error::DomainError;
use crate::sim_clock::{ClockStep, StepKind, session_close};
use crate::time::year_fraction;

/// Trading days per year: a replay step is one of them, so `trading_years` is 1/252.
const TRADING_DAYS_PER_YEAR: f64 = 252.0;

/// A clock that replays a list of trading dates, one per step.
///
/// The clock starts at the 16:00 New York close of the date at `start_index`. Each
/// [`advance`](EngineClock::advance) ignores the real time passed and moves to the close of the
/// next date, giving a step with
///
/// - `trading_years` = 1/252, one trading day of diffusion (a replay feed reads prices from a
///   table and does not diffuse, but the field keeps its meaning);
/// - `calendar_years` = the real ACT/365 gap between the two closes, so the step from Friday to
///   Monday carries three days of theta;
/// - `session_date` = the new date;
/// - `kind` = [`StepKind::Overnight`].
///
/// **Why `Overnight`.** The P&L attributor starts a new P&L day only on an overnight step, and
/// the engine fires corporate actions only on one. Every replay step is a whole day, so each
/// must do both, and a step has only one kind. The alternative, `Close`, would leave the P&L
/// running across days. The engine's own close-of-session records are therefore not produced
/// for a replay: the backtest runner reads each day's P&L from the snapshot instead.
///
/// `speed` is dates per second and is used only by whoever paces the replay. A paused clock, and
/// one that has reached the last date, return zero-length `Intraday` steps.
#[derive(Debug, Clone)]
pub struct ReplayClock {
    dates: Arc<[NaiveDate]>,
    index: usize,
    now: DateTime<Utc>,
    speed: f64,
    paused: bool,
}

impl ReplayClock {
    /// A clock over `dates` (strictly increasing) starting at `start_index`, at `speed` dates
    /// per second.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidReplay`] when `dates` is empty, not strictly increasing, or shorter
    /// than `start_index + 1`; [`DomainError::InvalidSpeed`] when `speed` is not finite and positive.
    pub fn new(
        dates: Arc<[NaiveDate]>,
        start_index: usize,
        speed: f64,
    ) -> Result<Self, DomainError> {
        if !(speed.is_finite() && speed > 0.0) {
            return Err(DomainError::InvalidSpeed(speed));
        }
        if dates.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(DomainError::InvalidReplay(
                "dates must be strictly increasing".to_owned(),
            ));
        }
        let start = dates.get(start_index).copied().ok_or_else(|| {
            DomainError::InvalidReplay(format!(
                "start index {start_index} is outside {} dates",
                dates.len()
            ))
        })?;
        Ok(Self {
            dates,
            index: start_index,
            now: session_close(start),
            speed,
            paused: false,
        })
    }

    /// The index of the current date.
    pub fn index(&self) -> usize {
        self.index
    }

    /// How many dates the replay holds.
    pub fn len(&self) -> usize {
        self.dates.len()
    }

    /// Whether the replay holds no dates. Never true for a clock that was built.
    pub fn is_empty(&self) -> bool {
        self.dates.is_empty()
    }

    /// The current date.
    pub fn date(&self) -> Option<NaiveDate> {
        self.dates.get(self.index).copied()
    }

    /// Whether the clock is on the last date, so no step is left.
    pub fn is_finished(&self) -> bool {
        self.index + 1 >= self.dates.len()
    }

    fn zero_step(&self) -> ClockStep {
        ClockStep {
            from: self.now,
            to: self.now,
            trading_years: 0.0,
            calendar_years: 0.0,
            session_date: self.date().unwrap_or(NaiveDate::MIN),
            kind: StepKind::Intraday,
        }
    }
}

impl Clock for ReplayClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

impl EngineClock for ReplayClock {
    fn advance(&mut self, _real_elapsed: Duration) -> ClockStep {
        if self.paused {
            return self.zero_step();
        }
        let Some(next) = self.dates.get(self.index + 1).copied() else {
            return self.zero_step();
        };
        let from = self.now;
        let to = session_close(next);
        self.index += 1;
        self.now = to;
        ClockStep {
            from,
            to,
            trading_years: 1.0 / TRADING_DAYS_PER_YEAR,
            calendar_years: year_fraction(from, to),
            session_date: next,
            kind: StepKind::Overnight,
        }
    }

    fn speed(&self) -> f64 {
        self.speed
    }

    fn set_speed(&mut self, speed: f64) -> Result<(), DomainError> {
        if !(speed.is_finite() && speed > 0.0) {
            return Err(DomainError::InvalidSpeed(speed));
        }
        self.speed = speed;
        Ok(())
    }

    fn pause(&mut self) {
        self.paused = true;
    }

    fn resume(&mut self) {
        self.paused = false;
    }

    fn is_paused(&self) -> bool {
        self.paused
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;

    fn date(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn clock() -> ReplayClock {
        // Thursday, Friday, Monday.
        let dates: Arc<[NaiveDate]> = vec![date(9, 24), date(9, 25), date(9, 28)].into();
        ReplayClock::new(dates, 0, 5.0).unwrap()
    }

    #[test]
    fn starts_at_the_close_of_the_start_date() {
        // 16:00 EDT is 20:00 UTC.
        assert_eq!(
            clock().now(),
            Utc.with_ymd_and_hms(2026, 9, 24, 20, 0, 0).unwrap()
        );
    }

    #[test]
    fn each_step_is_one_trading_day_and_the_real_calendar_gap() {
        let mut clock = clock();
        let step = clock.advance(Duration::ZERO);
        assert_eq!(step.kind, StepKind::Overnight);
        assert_eq!(step.session_date, date(9, 25));
        assert_relative_eq!(step.trading_years, 1.0 / 252.0);
        assert_relative_eq!(step.calendar_years, 1.0 / 365.0);
        // Friday to Monday carries the weekend.
        let weekend = clock.advance(Duration::from_secs(99));
        assert_eq!(weekend.session_date, date(9, 28));
        assert_relative_eq!(weekend.calendar_years, 3.0 / 365.0);
        assert_relative_eq!(weekend.trading_years, 1.0 / 252.0);
        assert!(clock.is_finished());
        assert_eq!(clock.index(), 2);
    }

    #[test]
    fn a_finished_or_paused_clock_stands_still() {
        let mut clock = clock();
        clock.pause();
        let frozen = clock.advance(Duration::ZERO);
        assert_eq!(frozen.from, frozen.to);
        assert_eq!(frozen.kind, StepKind::Intraday);
        assert_eq!(clock.index(), 0);
        clock.resume();
        clock.advance(Duration::ZERO);
        clock.advance(Duration::ZERO);
        let done = clock.advance(Duration::ZERO);
        assert_eq!(done.from, done.to);
        assert_relative_eq!(done.calendar_years, 0.0);
        assert_eq!(clock.index(), 2);
    }

    #[test]
    fn rejects_bad_dates_and_speeds() {
        let dates: Arc<[NaiveDate]> = vec![date(9, 25), date(9, 24)].into();
        assert!(matches!(
            ReplayClock::new(dates, 0, 1.0),
            Err(DomainError::InvalidReplay(_))
        ));
        let one: Arc<[NaiveDate]> = vec![date(9, 25)].into();
        assert!(ReplayClock::new(one.clone(), 1, 1.0).is_err());
        assert!(ReplayClock::new(one.clone(), 0, 0.0).is_err());
        let mut clock = ReplayClock::new(one, 0, 1.0).unwrap();
        assert!(clock.set_speed(f64::NAN).is_err());
        clock.set_speed(20.0).unwrap();
        assert_relative_eq!(clock.speed(), 20.0);
    }
}
