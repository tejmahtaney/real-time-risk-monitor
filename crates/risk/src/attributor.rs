//! [`PnlAttributor`]: turns a stream of per-tick position valuations into running P&L
//! attribution, per [`PnlBuckets`]'s formula.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use domain::{ClockStep, PositionId, StepKind, Symbol};
use portfolio::{PositionRisk, PositionStatus};

use crate::pnl::{PnlBuckets, PnlLevel, PnlReport, PreviousDay};

/// Turns a stream of per-tick position valuations into running P&L attribution.
///
/// Keeps the previous tick's [`PositionRisk`] per position id, so each call to
/// [`on_tick`](Self::on_tick) can diff against it and produce that tick's [`PnlBuckets`]
/// for every position, folded up to its underlying and to the portfolio. Call
/// [`report`](Self::report) at any point to read the running totals.
#[derive(Debug, Clone)]
pub struct PnlAttributor {
    previous: HashMap<PositionId, PositionRisk>,
    session_date: NaiveDate,
    portfolio: PnlLevel,
    by_underlying: BTreeMap<Symbol, PnlLevel>,
    by_position: BTreeMap<PositionId, PnlLevel>,
    previous_day: Option<PreviousDay>,
}

impl PnlAttributor {
    /// Starts attribution for a session opening on `session_date`.
    pub fn new(session_date: NaiveDate) -> Self {
        Self {
            previous: HashMap::new(),
            session_date,
            portfolio: PnlLevel::default(),
            by_underlying: BTreeMap::new(),
            by_position: BTreeMap::new(),
            previous_day: None,
        }
    }

    /// Attributes one tick's P&L from `positions`, the valuation at the end of `step`, against
    /// the valuation kept from the previous call.
    ///
    /// An [`Overnight`](StepKind::Overnight) step first rolls the day: today's totals at every
    /// level move to [`PreviousDay`] and today is reset, so the overnight step's own theta (and
    /// any other movement) lands in the new day, not the one that just closed. A position seen
    /// for the first time contributes nothing this tick, since there is nothing yet to diff
    /// against, and starts contributing from its next tick. A position that has just settled
    /// ([`PositionStatus::Expired`]) is attributed one last time against its settlement value
    /// and then dropped from tracking, keeping whatever it already contributed today and since
    /// start. A position that could not be valued ([`PositionStatus::Error`]) is skipped
    /// entirely: its numbers are zeroed and would not mean anything.
    pub fn on_tick(&mut self, positions: &[PositionRisk], step: &ClockStep) {
        if step.kind == StepKind::Overnight {
            self.roll_day(step.session_date);
        }
        let mut next_previous = HashMap::with_capacity(positions.len());
        for row in positions {
            if matches!(row.status, PositionStatus::Error { .. }) {
                continue;
            }
            if let Some(prev) = self.previous.get(&row.id) {
                let buckets = attribute_position(prev, row, step);
                self.accumulate(row.id.clone(), row.underlying.clone(), buckets);
            }
            if row.status == PositionStatus::Active {
                next_previous.insert(row.id.clone(), row.clone());
            }
        }
        self.previous = next_previous;
    }

    /// The current attribution report.
    pub fn report(&self) -> PnlReport {
        PnlReport {
            session_date: self.session_date,
            portfolio: self.portfolio,
            by_underlying: self.by_underlying.clone(),
            by_position: self.by_position.clone(),
            previous_day: self.previous_day.clone(),
        }
    }

    fn accumulate(&mut self, id: PositionId, underlying: Symbol, buckets: PnlBuckets) {
        let position = self.by_position.entry(id).or_default();
        position.today += buckets;
        position.since_start += buckets;
        let underlying_level = self.by_underlying.entry(underlying).or_default();
        underlying_level.today += buckets;
        underlying_level.since_start += buckets;
        self.portfolio.today += buckets;
        self.portfolio.since_start += buckets;
    }

    fn roll_day(&mut self, new_session_date: NaiveDate) {
        self.previous_day = Some(PreviousDay {
            session_date: self.session_date,
            portfolio: self.portfolio.today,
            by_underlying: self
                .by_underlying
                .iter()
                .map(|(symbol, level)| (symbol.clone(), level.today))
                .collect(),
            by_position: self
                .by_position
                .iter()
                .map(|(id, level)| (id.clone(), level.today))
                .collect(),
        });
        self.portfolio.today = PnlBuckets::default();
        for level in self.by_underlying.values_mut() {
            level.today = PnlBuckets::default();
        }
        for level in self.by_position.values_mut() {
            level.today = PnlBuckets::default();
        }
        self.session_date = new_session_date;
    }
}

/// Attributes one position's P&L for one tick. `prev` is the valuation at the start of the
/// tick (t₀), `curr` the valuation at the end (t₁). Uses `prev`'s Greeks throughout, per the
/// Taylor decomposition in [`PnlBuckets`].
///
/// The vol move dσ is forced to zero when `curr` has just settled: a settled row's `vol` is
/// zeroed (it no longer means anything once there is no time left), so diffing it against a
/// real `prev.vol` would invent a vol crash that never happened. Forcing dσ to zero there
/// leaves the option's whole time-value collapse in the residual, which is where the prompt
/// says it belongs: the residual is expected to grow near expiry.
fn attribute_position(prev: &PositionRisk, curr: &PositionRisk, step: &ClockStep) -> PnlBuckets {
    let scale = prev.quantity * prev.multiplier;
    let d_spot = curr.spot - prev.spot;
    let d_vol = if curr.status == PositionStatus::Expired {
        0.0
    } else {
        curr.vol - prev.vol
    };
    let dt = step.calendar_years;
    // The risk-free rate has no mutation path anywhere in this codebase today, so dr is
    // always zero and the rho bucket is always zero, exactly as the prompt anticipates.
    let dr = 0.0;
    let g = prev.greeks;

    let delta = g.delta * d_spot * scale;
    let gamma = 0.5 * g.gamma * d_spot * d_spot * scale;
    let vega = g.vega * d_vol * scale;
    let theta = g.theta * dt * scale;
    let rho = g.rho * dr * scale;
    let higher_order = (g.vanna * d_spot * d_vol + 0.5 * g.volga * d_vol * d_vol) * scale;
    let actual = (curr.mark - prev.mark) * scale;

    let mut buckets = PnlBuckets {
        delta,
        gamma,
        vega,
        theta,
        rho,
        higher_order,
        residual: 0.0,
        actual,
    };
    buckets.residual = actual - buckets.explained();
    buckets
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;
    use chrono::{TimeZone, Utc};
    use domain::{InstrumentKind, OptionType};
    use portfolio::ReportingGreeks;
    use pricing::{BsInputs, Greeks, greeks};

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn id(s: &str) -> PositionId {
        PositionId::new(s).unwrap()
    }

    /// A step carrying only `calendar_years`, `kind` and `session_date`: the only fields
    /// `attribute_position` and day-roll logic read. `from`/`to`/`trading_years` are unused.
    fn clock_step(calendar_years: f64, kind: StepKind, session_date: NaiveDate) -> ClockStep {
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap();
        ClockStep {
            from: now,
            to: now,
            trading_years: 0.0,
            calendar_years,
            session_date,
            kind,
        }
    }

    fn intraday(calendar_years: f64) -> ClockStep {
        clock_step(calendar_years, StepKind::Intraday, date(2026, 9, 28))
    }

    /// A long call, valued by full BSM revaluation so `mark` and `greeks` always agree, exactly
    /// as `portfolio::value_position` builds one. Strike 100, 4 % rate, no dividend yield,
    /// quantity 10 contracts: only `spot`, `vol` and `time_to_expiry` vary across the tests.
    fn base_call(spot: f64, vol: f64, time_to_expiry: f64) -> PositionRisk {
        let quantity = 10.0;
        let multiplier = 100.0;
        let inputs =
            BsInputs::new(spot, 100.0, time_to_expiry, 0.04, 0.0, vol).expect("valid inputs");
        let g = greeks(OptionType::Call, &inputs);
        PositionRisk {
            id: id("opt"),
            description: "opt".to_owned(),
            underlying: sym("AAA"),
            kind: InstrumentKind::Call,
            quantity,
            multiplier,
            spot,
            vol,
            time_to_expiry,
            mark: g.price,
            market_value: g.price * quantity * multiplier,
            entry_price: g.price,
            unrealised_pnl: 0.0,
            realised_pnl: 0.0,
            greeks: g,
            reporting: ReportingGreeks::scale(&g, quantity, multiplier, spot),
            status: PositionStatus::Active,
        }
    }

    fn stock_row(pid: &str, symbol: &str, quantity: f64, spot: f64) -> PositionRisk {
        let g = Greeks {
            price: spot,
            delta: 1.0,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            rho: 0.0,
            vanna: 0.0,
            volga: 0.0,
        };
        let multiplier = 1.0;
        PositionRisk {
            id: id(pid),
            description: pid.to_owned(),
            underlying: sym(symbol),
            kind: InstrumentKind::Equity,
            quantity,
            multiplier,
            spot,
            vol: 0.0,
            time_to_expiry: 0.0,
            mark: spot,
            market_value: spot * quantity * multiplier,
            entry_price: spot,
            unrealised_pnl: 0.0,
            realised_pnl: 0.0,
            greeks: g,
            reporting: ReportingGreeks::scale(&g, quantity, multiplier, spot),
            status: PositionStatus::Active,
        }
    }

    #[test]
    fn pure_time_step_theta_matches_actual_to_second_order() {
        let t0 = base_call(100.0, 0.20, 0.5);
        // One trading day of pure decay, spot and vol unchanged.
        let dt = 1.0 / 365.0;
        let t1 = base_call(100.0, 0.20, 0.5 - dt);
        let step = intraday(dt);
        let buckets = attribute_position(&t0, &t1, &step);

        assert_abs_diff_eq!(buckets.delta, 0.0);
        assert_abs_diff_eq!(buckets.gamma, 0.0);
        assert_abs_diff_eq!(buckets.vega, 0.0);
        assert_abs_diff_eq!(buckets.rho, 0.0);
        assert_abs_diff_eq!(buckets.higher_order, 0.0);
        assert_abs_diff_eq!(buckets.explained(), buckets.theta);
        // The residual is the O(dt^2) term Taylor-in-time leaves out: with dt of one day,
        // it should be a small fraction of the theta bucket itself.
        assert!(
            buckets.residual.abs() < 0.01 * buckets.theta.abs(),
            "residual {} vs theta {}",
            buckets.residual,
            buckets.theta
        );
    }

    #[test]
    fn spot_only_residual_shrinks_by_about_eight_when_the_move_halves() {
        let t0 = base_call(100.0, 0.20, 0.5);
        let step = intraday(0.0);

        // Halving dS should shrink the residual by about 2^3 = 8: the residual is the
        // O(dS^3) remainder of a Taylor expansion truncated after the dS^2 (gamma) term,
        // (1/6)*V'''(S0)*dS^3 plus higher terms, so it scales with the cube of the move.
        let full_move = base_call(105.0, 0.20, 0.5);
        let half_move = base_call(102.5, 0.20, 0.5);
        let full_residual = attribute_position(&t0, &full_move, &step).residual.abs();
        let half_residual = attribute_position(&t0, &half_move, &step).residual.abs();

        let ratio = full_residual / half_residual;
        assert!((6.0..=10.0).contains(&ratio), "ratio {ratio}");
    }

    #[test]
    fn vol_only_move_is_explained_by_vega_and_volga() {
        let t0 = base_call(100.0, 0.20, 0.5);
        let t1 = base_call(100.0, 0.21, 0.5);
        let step = intraday(0.0);
        let buckets = attribute_position(&t0, &t1, &step);

        assert_abs_diff_eq!(buckets.delta, 0.0);
        assert_abs_diff_eq!(buckets.gamma, 0.0);
        assert_abs_diff_eq!(buckets.theta, 0.0);
        assert_abs_diff_eq!(buckets.rho, 0.0);
        // vega + the volga half of higher_order should explain almost all of the move.
        assert!(
            buckets.residual.abs() < 0.05 * buckets.actual.abs(),
            "residual {} vs actual {}",
            buckets.residual,
            buckets.actual
        );
    }

    #[test]
    fn many_small_steps_leave_a_smaller_residual_than_one_big_step() {
        let start_spot = 100.0;
        let end_spot = 115.0;
        let vol = 0.20;
        let time_to_expiry = 0.5;
        let step = intraday(0.0);

        let one_big_step = {
            let t0 = base_call(start_spot, vol, time_to_expiry);
            let t1 = base_call(end_spot, vol, time_to_expiry);
            attribute_position(&t0, &t1, &step).residual.abs()
        };

        let n = 100;
        let mut total_small_steps = 0.0_f64;
        for i in 0..n {
            let s0 = start_spot + (end_spot - start_spot) * f64::from(i) / f64::from(n);
            let s1 = start_spot + (end_spot - start_spot) * f64::from(i + 1) / f64::from(n);
            let prev = base_call(s0, vol, time_to_expiry);
            let curr = base_call(s1, vol, time_to_expiry);
            total_small_steps += attribute_position(&prev, &curr, &step).residual.abs();
        }

        assert!(
            total_small_steps < one_big_step / 10.0,
            "small-step total {total_small_steps} vs one big step {one_big_step}"
        );
    }

    #[test]
    fn stock_only_book_has_zero_residual() {
        let t0 = stock_row("s", "AAA", 100.0, 100.0);
        let t1 = stock_row("s", "AAA", 100.0, 103.0);
        let step = intraday(1.0 / 365.0);
        let buckets = attribute_position(&t0, &t1, &step);

        assert_abs_diff_eq!(buckets.gamma, 0.0);
        assert_abs_diff_eq!(buckets.vega, 0.0);
        assert_abs_diff_eq!(buckets.theta, 0.0);
        assert_abs_diff_eq!(buckets.rho, 0.0);
        assert_abs_diff_eq!(buckets.higher_order, 0.0);
        assert_abs_diff_eq!(buckets.residual, 0.0);
        assert_abs_diff_eq!(buckets.delta, buckets.actual);
        assert_abs_diff_eq!(buckets.actual, 300.0);
    }

    #[test]
    fn an_iv_override_position_has_zero_vega_even_when_the_underlying_vol_moves() {
        // The override never moves, so `vol` is identical at t0 and t1 even though this
        // scenario stands in for the underlying's market vol having changed.
        let t0 = base_call(100.0, 0.65, 0.5);
        let t1 = base_call(103.0, 0.65, 0.5);
        let step = intraday(0.0);
        let buckets = attribute_position(&t0, &t1, &step);

        assert_abs_diff_eq!(buckets.vega, 0.0);
        assert_abs_diff_eq!(buckets.higher_order, 0.0);
    }

    #[test]
    fn buckets_add_up_at_every_level_after_several_ticks() {
        let mut attributor = PnlAttributor::new(date(2026, 9, 28));
        let step = intraday(1.0 / (365.0 * 24.0));

        let mut spot_a = 100.0;
        let mut spot_b = 50.0;
        attributor.on_tick(
            &[
                base_call(spot_a, 0.20, 0.5),
                stock_row("stk-b", "BBB", 200.0, spot_b),
            ],
            &step,
        );

        for _ in 0..5 {
            spot_a += 0.7;
            spot_b -= 0.3;
            let rows = [
                base_call(spot_a, 0.20, 0.5),
                stock_row("stk-b", "BBB", 200.0, spot_b),
            ];
            attributor.on_tick(&rows, &step);
        }

        let report = attributor.report();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6 * (1.0 + a.abs() + b.abs());

        let by_underlying_sum = report
            .by_underlying
            .values()
            .fold(PnlBuckets::default(), |acc, l| acc + l.since_start);
        let by_position_sum = report
            .by_position
            .values()
            .fold(PnlBuckets::default(), |acc, l| acc + l.since_start);

        assert!(close(
            report.portfolio.since_start.actual,
            by_underlying_sum.actual
        ));
        assert!(close(
            report.portfolio.since_start.actual,
            by_position_sum.actual
        ));
        assert!(close(
            report.portfolio.since_start.residual,
            by_position_sum.residual
        ));
        // Not the trivial all-zero case: the option leg has moved and left real numbers behind.
        assert!(report.portfolio.since_start.delta.abs() > 0.0);
    }

    #[test]
    fn overnight_step_rolls_today_into_previous_day_and_keeps_its_own_theta_in_the_new_day() {
        let day1 = date(2026, 9, 28);
        let day2 = date(2026, 9, 29);
        let mut attributor = PnlAttributor::new(day1);

        let intraday_step = intraday(1.0 / 365.0);
        attributor.on_tick(&[base_call(100.0, 0.20, 0.5)], &intraday_step);
        attributor.on_tick(&[base_call(100.0, 0.20, 0.5 - 1.0 / 365.0)], &intraday_step);
        let end_of_day1 = attributor.report().portfolio.today;

        // The overnight step: no spot or vol move, but real calendar time (a weekend-sized gap).
        let overnight_dt = 2.5 / 365.0;
        let overnight_step = clock_step(overnight_dt, StepKind::Overnight, day2);
        let before_overnight = base_call(100.0, 0.20, 0.5 - 1.0 / 365.0);
        let after_overnight = base_call(100.0, 0.20, 0.5 - 1.0 / 365.0 - overnight_dt);
        attributor.on_tick(std::slice::from_ref(&after_overnight), &overnight_step);

        let report = attributor.report();
        assert_eq!(report.session_date, day2);
        let previous = report.previous_day.expect("day should have rolled");
        assert_eq!(previous.session_date, day1);
        assert_abs_diff_eq!(
            previous.portfolio.actual,
            end_of_day1.actual,
            epsilon = 1e-9
        );

        // The new day's theta must include the overnight gap's theta and nothing from day 1.
        let overnight_only =
            attribute_position(&before_overnight, &after_overnight, &overnight_step);
        assert_abs_diff_eq!(
            report.portfolio.today.theta,
            overnight_only.theta,
            epsilon = 1e-9
        );
        assert_abs_diff_eq!(
            report.portfolio.since_start.actual,
            end_of_day1.actual + overnight_only.actual,
            epsilon = 1e-9
        );
    }

    #[test]
    fn a_position_added_mid_day_contributes_from_its_first_full_tick() {
        let mut attributor = PnlAttributor::new(date(2026, 9, 28));
        let step = intraday(0.0);

        attributor.on_tick(&[base_call(100.0, 0.20, 0.5)], &step);
        // "new" appears for the first time: no prior snapshot, so no bucket yet.
        attributor.on_tick(
            &[
                base_call(101.0, 0.20, 0.5),
                stock_row("new", "BBB", 10.0, 50.0),
            ],
            &step,
        );
        assert!(!attributor.report().by_position.contains_key(&id("new")));

        // Its first full tick: now it has a previous snapshot to diff against.
        attributor.on_tick(
            &[
                base_call(102.0, 0.20, 0.5),
                stock_row("new", "BBB", 10.0, 52.0),
            ],
            &step,
        );
        let report = attributor.report();
        let new_level = report.by_position[&id("new")];
        assert_abs_diff_eq!(new_level.since_start.actual, 20.0, epsilon = 1e-9);
    }

    #[test]
    fn a_removed_position_keeps_what_it_contributed_so_far_today() {
        let mut attributor = PnlAttributor::new(date(2026, 9, 28));
        let step = intraday(0.0);

        attributor.on_tick(&[stock_row("gone", "BBB", 10.0, 50.0)], &step);
        attributor.on_tick(&[stock_row("gone", "BBB", 10.0, 55.0)], &step);
        let before_removal = attributor.report().by_position[&id("gone")];
        assert_abs_diff_eq!(before_removal.since_start.actual, 50.0, epsilon = 1e-9);

        // The position is gone from the book: an empty tick.
        attributor.on_tick(&[], &step);
        let after_removal = attributor.report().by_position[&id("gone")];
        assert_abs_diff_eq!(
            after_removal.since_start.actual,
            before_removal.since_start.actual,
            epsilon = 1e-9
        );

        // It no longer contributes even if a later tick happens to hold the same id fresh
        // again: that would be treated as a brand new position with no diff on its own first tick.
        attributor.on_tick(&[stock_row("gone", "BBB", 10.0, 60.0)], &step);
        let after_reappearing = attributor.report().by_position[&id("gone")];
        assert_abs_diff_eq!(
            after_reappearing.since_start.actual,
            before_removal.since_start.actual,
            epsilon = 1e-9
        );
    }
}
