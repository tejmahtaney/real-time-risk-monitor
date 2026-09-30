//! The simulated market clock: US market hours only, with an overnight jump between sessions.

use std::time::Duration;

use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday,
};
use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::error::DomainError;
use crate::time::{MARKET_TZ, market_date, year_fraction};

/// Trading seconds in a year: 252 days of 6.5 hours. Vols are annualised over this.
pub const TRADING_SECONDS_PER_YEAR: f64 = 252.0 * 6.5 * 3_600.0;

/// What kind of move a [`ClockStep`] was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Time moved forward inside a session and stopped before the close.
    Intraday,
    /// Time moved forward and stopped exactly at 16:00. Options expiring today settle here.
    Close,
    /// The jump from a close to the next session's open. No trading time passes.
    Overnight,
}

/// One move of the simulated clock, carrying time on two scales.
///
/// The two clocks exist because two different conventions apply. Price diffusion uses
/// **trading time**: volatilities are quoted per √year of 252 trading days of 6.5 hours,
/// so a step's diffusion variance is σ² × `trading_years`, and nothing diffuses overnight
/// or at weekends. Option value decay uses **calendar time** (ACT/365): theta and time to
/// expiry run through the night and the weekend, so `calendar_years` is about 1e-6 for a
/// 30-second step but about 0.0075 for the step from Friday's close to Monday's open, which
/// puts the whole weekend of theta into Monday's first tick, as it lands on a real desk.
///
/// Fields are public so tests and the backtester can build steps by hand. `session_date`
/// is the New York date of the session the step belongs to; for an
/// [`Overnight`](StepKind::Overnight) step it is the session being opened.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClockStep {
    /// Simulated time at the start of the step.
    pub from: DateTime<Utc>,
    /// Simulated time at the end of the step.
    pub to: DateTime<Utc>,
    /// Trading seconds in the step divided by 252 × 6.5 × 3,600. Zero overnight.
    pub trading_years: f64,
    /// Calendar seconds in the step divided by 365 × 86,400 (ACT/365).
    pub calendar_years: f64,
    /// The New York date of the session this step belongs to.
    pub session_date: NaiveDate,
    /// Whether the step ended inside the session, at the close, or jumped overnight.
    pub kind: StepKind,
}

impl ClockStep {
    /// Builds the step from `from` to `to`, computing both year fractions. Trading time
    /// is zero for an overnight step.
    fn between(
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        session_date: NaiveDate,
        kind: StepKind,
    ) -> Self {
        let trading_years = match kind {
            StepKind::Overnight => 0.0,
            StepKind::Intraday | StepKind::Close => {
                let millis = to.signed_duration_since(from).num_milliseconds().max(0);
                millis as f64 / 1_000.0 / TRADING_SECONDS_PER_YEAR
            }
        };
        Self {
            from,
            to,
            trading_years,
            calendar_years: year_fraction(from, to),
            session_date,
            kind,
        }
    }
}

/// The instant at `hour`:`minute` New York time on `date`.
///
/// An ambiguous local time takes the earlier instant. A time inside a daylight saving gap
/// moves an hour on. Neither can happen at 09:30 or 16:00, since the changes are at 02:00,
/// but this keeps the clock free of a failure path.
fn new_york_instant(date: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(NaiveTime::MIN);
    let local = date.and_time(time);
    let resolved = MARKET_TZ
        .from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            MARKET_TZ
                .from_local_datetime(&(local + ChronoDuration::hours(1)))
                .earliest()
        });
    match resolved {
        Some(instant) => instant.with_timezone(&Utc),
        None => local.and_utc(),
    }
}

pub(crate) fn session_open(date: NaiveDate) -> DateTime<Utc> {
    new_york_instant(date, 9, 30)
}

fn session_close(date: NaiveDate) -> DateTime<Utc> {
    new_york_instant(date, 16, 0)
}

fn is_weekday(date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The first weekday strictly after `date`.
fn next_session_date(date: NaiveDate) -> NaiveDate {
    let mut next = date;
    loop {
        match next.succ_opt() {
            Some(day) => next = day,
            None => return next,
        }
        if is_weekday(next) {
            return next;
        }
    }
}

/// The session date `sessions` weekdays after `date`, so 0 gives `date` itself.
///
/// Exchange holidays are ignored, as everywhere in the simulated clock. Corporate-action
/// dates written as `+2d` count sessions from the simulation start with this.
///
/// ```
/// use chrono::NaiveDate;
/// use domain::add_sessions;
///
/// let thursday = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
/// // Friday is one session on, the following Monday is two.
/// assert_eq!(add_sessions(thursday, 2), NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
/// ```
pub fn add_sessions(date: NaiveDate, sessions: u32) -> NaiveDate {
    (0..sessions).fold(date, |day, _| next_session_date(day))
}

/// The first session open (09:30 New York, as a UTC instant) on a weekday strictly after `date`.
///
/// Exchange holidays are ignored, so a Friday gives the following Monday and Christmas
/// Day is treated as a trading day. The simulator's default start is this applied to the
/// date of the last real close.
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use domain::next_session_open_after;
///
/// let friday = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
/// // Monday 28 September 09:30 EDT is 13:30 UTC.
/// assert_eq!(
///     next_session_open_after(friday),
///     Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap()
/// );
/// ```
pub fn next_session_open_after(date: NaiveDate) -> DateTime<Utc> {
    session_open(next_session_date(date))
}

/// A simulated clock that runs only through US market hours.
///
/// A session is 09:30 to 16:00 New York time on a weekday. **Exchange holidays are
/// ignored**: every Monday to Friday is a session, including days such as Thanksgiving
/// when the real market is shut. Daylight saving is handled by working in New York local
/// time, so a session is always 6.5 trading hours, and the gap between sessions changes
/// by an hour across the spring and autumn changes.
///
/// Simulated time moves by real time × `speed`, rounded to whole milliseconds so that the
/// step's end time and both year fractions agree exactly. [`advance`](Self::advance)
/// never moves past 16:00: the step that reaches the close ends exactly there and the
/// excess is dropped. The call after that jumps to the next weekday 09:30 in one
/// [`Overnight`](StepKind::Overnight) step. The clock is synchronous and moves only when
/// told to, so tests and the Part 20 backtester drive it by hand.
#[derive(Debug, Clone)]
pub struct SimClock {
    now: DateTime<Utc>,
    session_date: NaiveDate,
    closed: bool,
    speed: f64,
    paused: bool,
}

impl SimClock {
    /// A clock that starts at `start` and runs at `speed` simulated seconds per real second.
    ///
    /// A `start` inside a weekday session is kept. A `start` before the open on a weekday
    /// moves to that day's open, and anything else (a weekend, or a weekday at or after the
    /// close) moves to the next weekday's open, so the clock always starts inside a session.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidSpeed`] when `speed` is not finite and positive.
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use domain::{Clock, SimClock};
    ///
    /// // Saturday 26 September 2026 moves to Monday's open, 13:30 UTC (09:30 EDT).
    /// let clock = SimClock::new(Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap(), 120.0)?;
    /// assert_eq!(clock.now(), Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap());
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(start: DateTime<Utc>, speed: f64) -> Result<Self, DomainError> {
        check_speed(speed)?;
        let date = market_date(start);
        let (now, session_date) = if is_weekday(date) && start < session_open(date) {
            (session_open(date), date)
        } else if is_weekday(date) && start < session_close(date) {
            (start, date)
        } else {
            let next = next_session_date(date);
            (session_open(next), next)
        };
        Ok(Self {
            now,
            session_date,
            closed: false,
            speed,
            paused: false,
        })
    }

    /// Simulated seconds per real second.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Changes the speed for later steps.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidSpeed`] when `speed` is not finite and positive. The old speed stays.
    pub fn set_speed(&mut self, speed: f64) -> Result<(), DomainError> {
        check_speed(speed)?;
        self.speed = speed;
        Ok(())
    }

    /// Stops time. [`advance`](Self::advance) returns zero-length steps until [`resume`](Self::resume).
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Lets time move again.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Whether the clock is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Moves simulated time on by `real_elapsed` × `speed` and reports the step.
    ///
    /// - Inside a session the step ends `real_elapsed × speed` later, rounded to the
    ///   millisecond, and is [`Intraday`](StepKind::Intraday). If that would reach or pass 16:00
    ///   the step ends exactly at 16:00 and is [`Close`](StepKind::Close).
    /// - After a close, the next call ignores `real_elapsed` and jumps to the next weekday's
    ///   09:30 as an [`Overnight`](StepKind::Overnight) step with zero trading time.
    /// - While paused the step has `from == to`, zero trading and calendar time, and kind
    ///   `Intraday`, even at a close.
    ///
    /// A `real_elapsed` so small that it rounds to zero milliseconds also gives a
    /// zero-length step.
    pub fn advance(&mut self, real_elapsed: Duration) -> ClockStep {
        if self.paused {
            return ClockStep::between(self.now, self.now, self.session_date, StepKind::Intraday);
        }
        if self.closed {
            return self.jump_to_next_open();
        }
        let close = session_close(self.session_date);
        let remaining_ms = close.signed_duration_since(self.now).num_milliseconds();
        let simulated_ms = (real_elapsed.as_secs_f64() * 1_000.0 * self.speed).round();
        let from = self.now;
        if simulated_ms < remaining_ms as f64 {
            // `simulated_ms` is non-negative and below `remaining_ms`, so it fits an i64.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let millis = simulated_ms as i64;
            self.now += ChronoDuration::milliseconds(millis);
            ClockStep::between(from, self.now, self.session_date, StepKind::Intraday)
        } else {
            self.now = close;
            self.closed = true;
            ClockStep::between(from, close, self.session_date, StepKind::Close)
        }
    }

    fn jump_to_next_open(&mut self) -> ClockStep {
        let from = self.now;
        let next = next_session_date(self.session_date);
        self.now = session_open(next);
        self.session_date = next;
        self.closed = false;
        ClockStep::between(from, self.now, next, StepKind::Overnight)
    }
}

impl Clock for SimClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn check_speed(speed: f64) -> Result<(), DomainError> {
    if speed.is_finite() && speed > 0.0 {
        Ok(())
    } else {
        Err(DomainError::InvalidSpeed(speed))
    }
}

#[cfg(test)]
mod tests {
    use approx::{assert_abs_diff_eq, assert_relative_eq};

    use super::*;

    const REAL_TICK: Duration = Duration::from_millis(250);
    const BIG: Duration = Duration::from_secs(1_000_000);

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Monday 28 September 2026, 09:30 EDT.
    fn monday_open() -> DateTime<Utc> {
        utc(2026, 9, 28, 13, 30)
    }

    fn hours(step: &ClockStep) -> f64 {
        step.to.signed_duration_since(step.from).num_seconds() as f64 / 3_600.0
    }

    #[test]
    fn a_full_session_ends_exactly_at_the_close() {
        // 250 ms at 120x is 30 simulated seconds, so 780 steps fill 6.5 hours.
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        let mut steps = Vec::new();
        loop {
            let step = clock.advance(REAL_TICK);
            let kind = step.kind;
            steps.push(step);
            if kind == StepKind::Close {
                break;
            }
            assert!(steps.len() < 10_000, "session never closed");
        }
        assert_eq!(steps.len(), 780);
        for pair in steps.windows(2) {
            assert_eq!(pair[0].to, pair[1].from, "steps must chain");
        }
        assert!(steps[..779].iter().all(|s| s.kind == StepKind::Intraday));
        let last = steps.last().unwrap();
        assert_eq!(last.to, utc(2026, 9, 28, 20, 0));
        assert_eq!(clock.now(), utc(2026, 9, 28, 20, 0));
        assert_eq!(last.session_date, date(2026, 9, 28));
        let trading: f64 = steps.iter().map(|s| s.trading_years).sum();
        assert_relative_eq!(trading, 1.0 / 252.0, max_relative = 1e-12);
        let calendar: f64 = steps.iter().map(|s| s.calendar_years).sum();
        assert_relative_eq!(
            calendar,
            6.5 * 3_600.0 / (365.0 * 86_400.0),
            max_relative = 1e-12
        );
        // One intraday step is 30 s of trading time and of calendar time.
        assert_relative_eq!(steps[0].trading_years, 30.0 / TRADING_SECONDS_PER_YEAR);
        assert_relative_eq!(steps[0].calendar_years, 30.0 / (365.0 * 86_400.0));
    }

    #[test]
    fn an_oversize_step_is_clamped_to_the_close() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        // 1 real hour is 120 simulated hours.
        let step = clock.advance(Duration::from_secs(3_600));
        assert_eq!(step.kind, StepKind::Close);
        assert_eq!(step.to, utc(2026, 9, 28, 20, 0));
        assert_relative_eq!(step.trading_years, 1.0 / 252.0, max_relative = 1e-12);
    }

    #[test]
    fn the_step_after_a_close_jumps_to_the_next_open() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        assert_eq!(clock.advance(BIG).kind, StepKind::Close);
        let step = clock.advance(REAL_TICK);
        assert_eq!(step.kind, StepKind::Overnight);
        assert_eq!(step.from, utc(2026, 9, 28, 20, 0));
        assert_eq!(step.to, utc(2026, 9, 29, 13, 30));
        assert_eq!(step.session_date, date(2026, 9, 29));
        assert_abs_diff_eq!(step.trading_years, 0.0);
        // 17.5 hours.
        assert_relative_eq!(step.calendar_years, 17.5 * 3_600.0 / (365.0 * 86_400.0));
        // The session then runs normally from the open.
        let next = clock.advance(REAL_TICK);
        assert_eq!(next.kind, StepKind::Intraday);
        assert_eq!(next.from, utc(2026, 9, 29, 13, 30));
    }

    #[test]
    fn friday_close_jumps_to_monday_with_the_weekend_gap() {
        let friday_open = utc(2026, 9, 25, 13, 30);
        let mut clock = SimClock::new(friday_open, 120.0).unwrap();
        assert_eq!(clock.advance(BIG).kind, StepKind::Close);
        let step = clock.advance(REAL_TICK);
        assert_eq!(step.kind, StepKind::Overnight);
        assert_eq!(step.from, utc(2026, 9, 25, 20, 0));
        assert_eq!(step.to, utc(2026, 9, 28, 13, 30));
        assert_eq!(step.session_date, date(2026, 9, 28));
        assert_abs_diff_eq!(step.trading_years, 0.0);
        // Friday 16:00 to Monday 09:30 is 65.5 hours: about 0.0075 years.
        assert_relative_eq!(hours(&step), 65.5);
        assert_relative_eq!(step.calendar_years, 65.5 * 3_600.0 / (365.0 * 86_400.0));
    }

    #[test]
    fn sessions_run_monday_to_friday_only() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        let mut opened = Vec::new();
        for _ in 0..6 {
            assert_eq!(clock.advance(BIG).kind, StepKind::Close);
            opened.push(clock.advance(BIG).session_date);
        }
        let expected = [
            date(2026, 9, 29),
            date(2026, 9, 30),
            date(2026, 10, 1),
            date(2026, 10, 2),
            date(2026, 10, 5),
            date(2026, 10, 6),
        ];
        assert_eq!(opened, expected);
    }

    #[test]
    fn spring_forward_shortens_the_weekend() {
        // Clocks go forward on Sunday 8 March 2026. Friday closes at 16:00 EST (21:00 UTC),
        // Monday opens at 09:30 EDT (13:30 UTC): one hour less than a normal weekend.
        let mut clock = SimClock::new(utc(2026, 3, 6, 15, 0), 120.0).unwrap();
        let close = clock.advance(BIG);
        assert_eq!(close.to, utc(2026, 3, 6, 21, 0));
        let overnight = clock.advance(BIG);
        assert_eq!(overnight.to, utc(2026, 3, 9, 13, 30));
        assert_relative_eq!(hours(&overnight), 64.5);
        assert_relative_eq!(
            overnight.calendar_years,
            64.5 * 3_600.0 / (365.0 * 86_400.0)
        );
        // The session is still 6.5 trading hours.
        let monday = clock.advance(BIG);
        assert_eq!(monday.kind, StepKind::Close);
        assert_eq!(monday.to, utc(2026, 3, 9, 20, 0));
        assert_relative_eq!(hours(&monday), 6.5);
        assert_relative_eq!(monday.trading_years, 1.0 / 252.0, max_relative = 1e-12);
    }

    #[test]
    fn fall_back_lengthens_the_weekend() {
        // Clocks go back on Sunday 1 November 2026. Friday closes at 16:00 EDT (20:00 UTC),
        // Monday opens at 09:30 EST (14:30 UTC): one hour more than a normal weekend.
        let mut clock = SimClock::new(utc(2026, 10, 30, 14, 0), 120.0).unwrap();
        let close = clock.advance(BIG);
        assert_eq!(close.to, utc(2026, 10, 30, 20, 0));
        let overnight = clock.advance(BIG);
        assert_eq!(overnight.to, utc(2026, 11, 2, 14, 30));
        assert_relative_eq!(hours(&overnight), 66.5);
        let monday = clock.advance(BIG);
        assert_eq!(monday.to, utc(2026, 11, 2, 21, 0));
        assert_relative_eq!(hours(&monday), 6.5);
    }

    #[test]
    fn next_open_after_a_date() {
        // Friday and Saturday both give Monday, Monday gives Tuesday.
        let monday = utc(2026, 9, 28, 13, 30);
        assert_eq!(next_session_open_after(date(2026, 9, 25)), monday);
        assert_eq!(next_session_open_after(date(2026, 9, 26)), monday);
        assert_eq!(next_session_open_after(date(2026, 9, 27)), monday);
        assert_eq!(
            next_session_open_after(date(2026, 9, 28)),
            utc(2026, 9, 29, 13, 30)
        );
        // Across the autumn change the open moves from 13:30 to 14:30 UTC.
        assert_eq!(
            next_session_open_after(date(2026, 10, 30)),
            utc(2026, 11, 2, 14, 30)
        );
    }

    #[test]
    fn the_start_is_snapped_into_a_session() {
        let start = |t| SimClock::new(t, 1.0).unwrap().now();
        // Inside a session: kept.
        assert_eq!(start(utc(2026, 9, 28, 15, 12)), utc(2026, 9, 28, 15, 12));
        // Before the open: that day's open. 12:00 UTC is 08:00 EDT.
        assert_eq!(start(utc(2026, 9, 28, 12, 0)), monday_open());
        // At or after the close: the next open.
        assert_eq!(start(utc(2026, 9, 28, 20, 0)), utc(2026, 9, 29, 13, 30));
        assert_eq!(start(utc(2026, 9, 28, 23, 0)), utc(2026, 9, 29, 13, 30));
        // Weekend: Monday's open.
        assert_eq!(start(utc(2026, 9, 26, 15, 0)), monday_open());
        assert_eq!(start(utc(2026, 9, 27, 15, 0)), monday_open());
        // 01:00 UTC on Saturday is still Friday evening in New York, after the close.
        assert_eq!(start(utc(2026, 9, 26, 1, 0)), monday_open());
    }

    #[test]
    fn a_mid_session_start_has_the_right_session_date() {
        let mut clock = SimClock::new(utc(2026, 9, 28, 19, 59), 60.0).unwrap();
        // One real second is one simulated minute, which reaches the close.
        let step = clock.advance(Duration::from_secs(1));
        assert_eq!(step.kind, StepKind::Close);
        assert_eq!(step.session_date, date(2026, 9, 28));
        assert_relative_eq!(step.trading_years, 60.0 / TRADING_SECONDS_PER_YEAR);
    }

    #[test]
    fn pause_stops_time_and_resume_restarts_it() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        clock.pause();
        assert!(clock.is_paused());
        let frozen = clock.advance(BIG);
        assert_eq!(frozen.kind, StepKind::Intraday);
        assert_eq!(frozen.from, frozen.to);
        assert_abs_diff_eq!(frozen.trading_years, 0.0);
        assert_abs_diff_eq!(frozen.calendar_years, 0.0);
        assert_eq!(clock.now(), monday_open());
        clock.resume();
        assert!(!clock.is_paused());
        assert_eq!(
            clock.advance(REAL_TICK).to,
            monday_open() + ChronoDuration::seconds(30)
        );
    }

    #[test]
    fn a_paused_clock_does_not_skip_the_overnight_jump() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        assert_eq!(clock.advance(BIG).kind, StepKind::Close);
        clock.pause();
        let frozen = clock.advance(BIG);
        assert_eq!(frozen.from, frozen.to);
        clock.resume();
        assert_eq!(clock.advance(BIG).kind, StepKind::Overnight);
    }

    #[test]
    fn set_speed_changes_the_next_step() {
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        let slow = clock.advance(REAL_TICK);
        assert_relative_eq!(hours(&slow) * 3_600.0, 30.0);
        clock.set_speed(240.0).unwrap();
        assert_relative_eq!(clock.speed(), 240.0);
        let fast = clock.advance(REAL_TICK);
        assert_relative_eq!(hours(&fast) * 3_600.0, 60.0);
    }

    #[test]
    fn bad_speeds_are_rejected() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                SimClock::new(monday_open(), bad),
                Err(DomainError::InvalidSpeed(_))
            ));
        }
        let mut clock = SimClock::new(monday_open(), 120.0).unwrap();
        assert!(clock.set_speed(-5.0).is_err());
        assert_relative_eq!(clock.speed(), 120.0);
    }

    #[test]
    fn time_is_rounded_to_whole_milliseconds() {
        // 1 ms at 1.5x is 1.5 ms, which rounds to 2 ms.
        let mut clock = SimClock::new(monday_open(), 1.5).unwrap();
        let step = clock.advance(Duration::from_millis(1));
        assert_eq!(step.to - step.from, ChronoDuration::milliseconds(2));
        assert_relative_eq!(step.calendar_years, 0.002 / (365.0 * 86_400.0));
    }

    #[test]
    fn step_kind_serialises_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&StepKind::Overnight).unwrap(),
            "\"overnight\""
        );
    }
}
