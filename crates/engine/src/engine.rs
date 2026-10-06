//! `Engine`: owns all mutable state, ticks the feed, values the book, publishes
//! snapshots. Nothing else touches the book, the feed or the clock, so there are no
//! locks on the hot path.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::AtomicU64;

use chrono::{NaiveDate, Utc};
use domain::{
    AppliedAction, Clock, ClockStep, EngineClock, MarketState, Position, SimClock, StepKind,
    Symbol, market_date,
};
use marketdata::{CorporateActionCalendar, PriceFeed, SeedData};
use portfolio::{
    PortfolioError, PositionBook, PositionSpec, SettlementEvent, adjust_specs_for_ledger,
    ledger_splits, resolve, to_spec,
};
use risk::{
    AlertEvent, LimitMonitor, LimitStatus, LimitsConfig, PnlAttributor, VarCalculator, VarMethod,
    aggregate,
};
use storage::StorageRoot;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{Instant as TokioInstant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::backtest::{BacktestRollEvent, RollPolicy};
use crate::command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
use crate::corporate::{CorporateActionEvent, CorporateActionState, missing_splits};
use crate::error::EngineError;
use crate::handle::EngineHandle;
use crate::recorder::{
    self, ControlAction, PositionChangeKind, RecorderRecord, SessionEvent, SessionMeta,
};
use crate::snapshot::{FeedStatus, HistoryPoint, RiskSnapshot, SymbolSnapshot};

/// Lower bound for [`EngineCommand::SetSpeed`], inclusive.
const MIN_SPEED: f64 = 1.0;
/// Upper bound for [`EngineCommand::SetSpeed`], inclusive.
const MAX_SPEED: f64 = 3600.0;

/// Owns the position book, price feed, clock, market state, P&L attributor and limit
/// monitor. Generic over the feed so tests can plug in a fake that fails on command;
/// production code instantiates `Engine<marketdata::GbmSimulator>` once for the live app. The
/// backtester builds `Engine<marketdata::ReplayFeed, domain::ReplayClock>`: the clock is a type
/// parameter too, so a replay runs exactly this tick code.
pub struct Engine<F: PriceFeed, C: EngineClock = SimClock> {
    clock: C,
    pub(crate) feed: F,
    pub(crate) market: MarketState,
    pub(crate) book: PositionBook,
    pub(crate) attributor: PnlAttributor,
    monitor: LimitMonitor,
    /// Value at Risk (Part 18): the session's calibrated correlation/return data, the
    /// delta-gamma Monte Carlo's cached common random numbers, and the adaptive compute
    /// budget.
    var_calculator: VarCalculator,
    /// Each symbol's spot at the start of the current session, for `change_since_open`.
    open_spots: BTreeMap<Symbol, f64>,
    seq: u64,
    last_snapshot: Arc<RiskSnapshot>,
    /// Alert events from the most recent call to `tick`, drained by `spawn`'s loop onto
    /// the broadcast channel.
    pub(crate) new_alerts: Vec<AlertEvent>,
    /// Bounded ring buffer of per-tick history points, oldest first, for charting.
    history: VecDeque<HistoryPoint>,
    /// Maximum number of points [`Self::history`] is allowed to hold.
    history_capacity: usize,
    /// This session's id: a UTC start time plus a short random suffix.
    session_id: String,
    /// This session's metadata, handed to the recorder task in [`Self::spawn`]. The
    /// engine itself never writes it to disk — see [`crate::recorder`]'s doc comment.
    session_meta: SessionMeta,
    /// Records this tick or command produced, drained onto the recorder's channel by
    /// [`Self::spawn`]'s loop after each `tick`/`apply` call.
    pub(crate) pending_records: Vec<RecorderRecord>,
    /// Corporate actions still to fire.
    pub(crate) calendar: CorporateActionCalendar,
    /// Corporate actions already applied, oldest first. Grows as actions fire.
    pub(crate) ledger: Vec<AppliedAction>,
    /// The most recent corporate-action events, carried on every snapshot.
    pub(crate) corporate_events: VecDeque<CorporateActionEvent>,
    /// What to do with an option that has just settled. `None` in live mode; the backtester sets
    /// one to keep the book alive through a replay.
    roll_policy: Option<Box<dyn RollPolicy>>,
    /// Positions already past their expiry when the engine was built, settled before tick 0.
    initial_settlements: Vec<SettlementEvent>,
}

impl<F: PriceFeed, C: EngineClock> Engine<F, C> {
    /// Builds an engine and publishes its first ("tick 0") snapshot immediately: resolves
    /// the starting positions against the seed's market state, seeds the P&L attributor
    /// with the session's opening valuation via a zero-length `ClockStep` (so the first
    /// real tick has something to diff against instead of being treated as a fresh add
    /// for every position), and evaluates limits once so the initial snapshot's active
    /// alerts aren't empty.
    ///
    /// `clock` is built by the caller exactly as `main` and the fixed-seed tests already
    /// do (`SimClock::new(sim_config.start_time(&seed), sim_config.speed)?`), so clock
    /// construction and its own speed validation stay at the one call site that already
    /// does it. `wall_clock` supplies the real time stamped on `LimitMonitor`'s alert
    /// events; production code passes a real clock, tests a fixed one. `history_capacity`
    /// bounds the history ring buffer (see [`Self::history_since`]); `0` disables it.
    /// `rng_seed` is `config/simulation.toml`'s RNG seed — a different thing from
    /// `seed: &SeedData`, the calibrated *market* seed above — recorded in `meta.json`.
    /// `headline_var_method` is `config/risk.toml`'s `headline_var_method` (Part 18):
    /// which of the three `VaR` methods' `VaR99` backs the headline figure, `HistoryPoint`
    /// and the `var_99` limit.
    ///
    /// This constructor performs no I/O: it computes this session's id and metadata
    /// (see [`Self::session_id`]) but does not write them anywhere. [`Self::spawn`]
    /// hands them to the recorder task, which does the actual writing — see
    /// [`crate::recorder`]'s doc comment for why.
    ///
    /// # Errors
    ///
    /// [`EngineError::Calibration`] if the seed cannot build a market state,
    /// [`EngineError::Portfolio`] if the starting positions cannot be resolved or added
    /// to the book.
    // Nine constructor parameters is over clippy's default of seven, but every one is
    // a distinct, already-validated piece of state this type owns; grouping them into
    // a builder or config struct would only move the same nine names one level down.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: C,
        seed: &SeedData,
        feed: F,
        specs: &[PositionSpec],
        limits: LimitsConfig,
        wall_clock: Box<dyn Clock>,
        history_capacity: usize,
        rng_seed: u64,
        headline_var_method: VarMethod,
    ) -> Result<Self, EngineError> {
        Self::new_with_actions(
            clock,
            seed,
            feed,
            specs,
            limits,
            wall_clock,
            history_capacity,
            rng_seed,
            headline_var_method,
            CorporateActionState::default(),
        )
    }

    /// [`Self::new`] with corporate actions (Part 19).
    ///
    /// `actions.calendar` is what still has to fire and `actions.ledger` what has already
    /// been applied. Every position loaded here is first brought up to date with the ledger:
    /// for each ledgered split on its symbol that the position does not already list in its
    /// `applied_actions`, the split is applied and its id recorded (see
    /// [`portfolio::adjust_specs_for_ledger`]). The caller is expected to have replayed the
    /// same ledger into `seed`, so the market and the book start in the same terms.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_actions(
        clock: C,
        seed: &SeedData,
        feed: F,
        specs: &[PositionSpec],
        limits: LimitsConfig,
        wall_clock: Box<dyn Clock>,
        history_capacity: usize,
        rng_seed: u64,
        headline_var_method: VarMethod,
        actions: CorporateActionState,
    ) -> Result<Self, EngineError> {
        let started = Instant::now();
        let start = clock.now();
        let session_date = market_date(start);

        let market = seed.to_market_state(start)?;
        let specs = adjust_specs_for_ledger(specs.to_vec(), &ledger_splits(&actions.ledger));
        let positions = resolve(&specs, &market, start)?;
        let mut book = PositionBook::from_positions(positions)?;
        // Settle anything already past its expiry before the very first valuation, so a
        // restored book (e.g. `start_portfolio = "last"` loaded after the server was
        // off overnight) never shows a stale, unsettled error row on tick 0. The
        // recorder task does not exist yet at this point, so these settlements are
        // logged but not written to events.jsonl — an accepted, documented edge case.
        let initial_settlements = book.settle_expired(start, &market);
        for event in &initial_settlements {
            tracing::info!(id = %event.position_id, "settled an already-expired position at session start");
        }

        // The session's own start/end are real wall-clock times (when the server
        // process actually started and stopped), not simulated market time — `start`
        // above is the sim clock's opening instant and is unrelated. Read directly from
        // `Utc::now()` rather than `wall_clock` (which a test may fix to a simulated
        // date for deterministic alert timestamps), so this always matches the real
        // time [`crate::recorder::run`] stamps on the session's final `meta.json`.
        let session_start = Utc::now();
        let session_id = recorder::new_session_id(session_start);
        let session_meta = SessionMeta {
            session_id: session_id.clone(),
            start: session_start,
            end: None,
            seed: rng_seed,
            speed: clock.speed(),
            git_commit: recorder::git_commit(),
            dropped_records: 0,
        };

        let mut attributor = PnlAttributor::new(session_date);
        let opening_step = ClockStep {
            from: start,
            to: start,
            trading_years: 0.0,
            calendar_years: 0.0,
            session_date,
            kind: StepKind::Intraday,
        };
        attributor.on_tick(&book.value_all(&market, start), &opening_step);

        let mut monitor = LimitMonitor::new(limits, wall_clock);
        let rows = book.value_all(&market, start);
        let summary = aggregate(&rows);

        let mut var_calculator = VarCalculator::new(
            seed.symbols.clone(),
            seed.correlation.clone(),
            seed.returns.clone(),
            seed.return_dates.clone(),
            headline_var_method,
        );
        let var_report = var_calculator.compute(&summary, &market, &book, start);
        let headline_var_99 = var_report.headline().var_99;

        let _events = monitor.evaluate(&summary, headline_var_99, start);
        let limit_statuses = monitor.statuses(&summary, headline_var_99);

        let open_spots = session_open_spots(&market);
        let pnl = attributor.report();
        let active_alerts: Vec<LimitStatus> = monitor.active().cloned().collect();
        let alert_history: Vec<AlertEvent> = monitor.history().cloned().collect();

        #[allow(clippy::cast_possible_truncation)]
        let compute_micros = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;

        let snapshot = build_snapshot(
            0,
            Utc::now(),
            &opening_step,
            &market,
            &open_spots,
            rows,
            summary,
            pnl,
            active_alerts,
            limit_statuses,
            alert_history,
            clock.speed(),
            clock.is_paused(),
            FeedStatus::Ok,
            compute_micros,
            var_report,
            Vec::new(),
        );

        let mut history = VecDeque::new();
        push_history(&mut history, history_capacity, history_point(&snapshot));

        Ok(Self {
            clock,
            feed,
            market,
            book,
            attributor,
            monitor,
            var_calculator,
            open_spots,
            seq: 0,
            last_snapshot: Arc::new(snapshot),
            new_alerts: Vec::new(),
            history,
            history_capacity,
            session_id,
            session_meta,
            pending_records: Vec::new(),
            calendar: actions.calendar,
            ledger: actions.ledger,
            corporate_events: VecDeque::new(),
            roll_policy: None,
            initial_settlements,
        })
    }

    /// Sets what happens to an option that settles: the policy may name a replacement, which is
    /// resolved against the market at that moment and added to the book before the tick's
    /// valuation. Used by the backtester; live mode has none.
    #[must_use]
    pub fn with_roll_policy(mut self, policy: Box<dyn RollPolicy>) -> Self {
        self.roll_policy = Some(policy);
        self
    }

    /// Replaces the `VaR` calculator's correlation and historical return window. The backtester
    /// calls this before each tick with data up to that tick's date.
    pub(crate) fn set_var_window(
        &mut self,
        correlation: Vec<Vec<f64>>,
        returns: Vec<Vec<f64>>,
        return_dates: Vec<NaiveDate>,
    ) {
        self.var_calculator
            .set_window(correlation, returns, return_dates);
    }

    /// Makes `VaR` compute on every tick, never throttled. See [`VarCalculator::compute_every_tick`].
    pub(crate) fn compute_var_every_tick(&mut self) {
        self.var_calculator.compute_every_tick();
    }

    /// The clock.
    pub(crate) fn clock(&self) -> &C {
        &self.clock
    }

    /// The clock, mutably.
    pub(crate) fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    /// The most recently published snapshot.
    pub(crate) fn latest_snapshot(&self) -> Arc<RiskSnapshot> {
        self.last_snapshot.clone()
    }

    /// The options that were already expired when the engine was built and settled before tick 0.
    pub(crate) fn initial_settlements(&self) -> &[SettlementEvent] {
        &self.initial_settlements
    }

    /// Asks the roll policy for a replacement for each settled option and adds it, recording a
    /// [`BacktestRollEvent`]. Does nothing without a policy. A replacement that cannot be added is
    /// logged and skipped.
    ///
    /// Called inside a tick, before that tick's valuation. A position rolled at the close of date
    /// d is therefore in the valuation, risk and `VaR` computed at that close. That `VaR` is the
    /// start-of-day figure for d + 1, so the new option first appears in the `VaR` used for d + 1;
    /// the `VaR` used for d was computed a day earlier and does not hold it. The attributor sees the
    /// new position for the first time on this tick, so it contributes P&L from d + 1.
    fn roll_settled(&mut self, settlements: &[SettlementEvent], date: NaiveDate) {
        let Some(mut policy) = self.roll_policy.take() else {
            return;
        };
        for settled in settlements {
            let Some(spec) = policy.replacement(settled, date) else {
                continue;
            };
            let opened = spec.id.clone();
            match self.add_position(&spec) {
                Ok(EngineReply::Added(row)) => {
                    tracing::info!(settled = %settled.position_id, opened = %opened, "rolled an expired option");
                    self.pending_records
                        .push(RecorderRecord::Roll(BacktestRollEvent {
                            sim_time: self.clock.now(),
                            session_date: date,
                            settled: settled.position_id.clone(),
                            opened,
                            description: row.description.clone(),
                        }));
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(settled = %settled.position_id, %error, "could not roll an expired option");
                }
            }
        }
        self.roll_policy = Some(policy);
    }

    /// This session's id: a UTC start time plus a short random suffix.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Positions currently in the book that have not settled.
    ///
    /// A settled option's cash is already paid out and its outcome is recorded in
    /// `events.jsonl`, so it must never come back to life on a restart that resolves
    /// the same book again — this is what "the book" means for
    /// `portfolios/current.json` and a saved portfolio version.
    pub fn active_positions(&self) -> Vec<Position> {
        self.book
            .iter()
            .filter(|p| self.book.settlement(&p.id).is_none())
            .cloned()
            .collect()
    }

    /// [`Self::active_positions`], converted to [`PositionSpec`] via [`portfolio::to_spec`].
    pub fn active_specs(&self) -> Vec<PositionSpec> {
        self.active_positions().iter().map(to_spec).collect()
    }

    /// One full cycle: advance the clock, step the feed, settle expiries, value
    /// positions, attribute P&L, aggregate, compute `VaR` (Part 18), evaluate limits,
    /// build and publish a snapshot.
    ///
    /// A feed failure means stale prices, not stopped time: if `feed.step` or applying
    /// its tick to the market fails, both are documented to leave their state unchanged,
    /// so `self.market` keeps exactly the last tick's spot and vol. Every later step
    /// (settlement, valuation, attribution, limits) still runs against that stale market
    /// and the real time that has passed, so an option still settles at its expiry, an
    /// overnight step still rolls the P&L day, and theta still accrues, regardless of
    /// feed health. Only `feed_status` on the resulting snapshot changes.
    ///
    /// Always returns `Ok`; no path in this method constructs an `EngineError` today
    /// (see that type's doc comment for why).
    ///
    /// # Errors
    ///
    /// Never returns `Err` in the current implementation.
    pub fn tick(&mut self, real_elapsed: Duration) -> Result<Arc<RiskSnapshot>, EngineError> {
        let started = Instant::now();
        let step = self.clock.advance(real_elapsed);
        // The market exactly as the previous tick left it, before this tick's feed step and
        // before any corporate action. Only an overnight step can open a session with an
        // action due, so only then is the clone taken. A split rebases the attributor
        // against this (see `apply_due_actions`).
        let market_before_tick = (step.kind == StepKind::Overnight).then(|| self.market.clone());

        let feed_status = match self.feed.step(&step) {
            Ok(tick) => match tick.apply_to(&mut self.market) {
                Ok(()) => FeedStatus::Ok,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        "applying the feed's tick to the market failed; market prices are unchanged, risk pipeline still advances on stale prices"
                    );
                    FeedStatus::Degraded
                }
            },
            Err(err) => {
                tracing::error!(
                    error = %err,
                    "feed step failed; market prices are unchanged, risk pipeline still advances on stale prices"
                );
                FeedStatus::Degraded
            }
        };

        if let Some(previous) = market_before_tick {
            // Actions apply at the open, before anything below values the book. The open spots
            // are recorded after them, so a 4-for-1 does not show as a 75 % fall since open.
            self.apply_due_actions(&step, previous);
            self.open_spots = session_open_spots(&self.market);
        }

        let settlements = self.book.settle_expired(step.to, &self.market);
        if !settlements.is_empty() {
            for event in &settlements {
                self.pending_records
                    .push(RecorderRecord::Event(SessionEvent::Settlement(
                        event.clone(),
                    )));
            }
            // Rolled before valuation, so the new option is in this tick's valuation, risk and VaR.
            // That VaR is computed at this close and is the start-of-day figure for the *next* date:
            // an option rolled at the close of d first appears in the VaR used for d + 1. The VaR
            // used for d itself was computed a day earlier and never held the new option.
            self.roll_settled(&settlements, step.session_date);
            self.pending_records
                .push(RecorderRecord::CurrentPortfolio(self.active_positions()));
        }
        let rows = self.book.value_all(&self.market, step.to);
        self.attributor.on_tick(&rows, &step);
        let summary = aggregate(&rows);

        let var_report = self
            .var_calculator
            .compute(&summary, &self.market, &self.book, step.to);
        let headline_var_99 = var_report.headline().var_99;

        let events = self.monitor.evaluate(&summary, headline_var_99, step.to);
        for event in &events {
            self.pending_records
                .push(RecorderRecord::Event(SessionEvent::Alert(event.clone())));
        }
        self.new_alerts.extend(events);
        let limit_statuses = self.monitor.statuses(&summary, headline_var_99);

        self.seq += 1;
        let pnl = self.attributor.report();
        let active_alerts: Vec<LimitStatus> = self.monitor.active().cloned().collect();
        let alert_history: Vec<AlertEvent> = self.monitor.history().cloned().collect();

        #[allow(clippy::cast_possible_truncation)]
        let compute_micros = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;

        let snapshot = build_snapshot(
            self.seq,
            Utc::now(),
            &step,
            &self.market,
            &self.open_spots,
            rows,
            summary,
            pnl,
            active_alerts,
            limit_statuses,
            alert_history,
            self.clock.speed(),
            self.clock.is_paused(),
            feed_status,
            compute_micros,
            var_report,
            self.corporate_events.iter().cloned().collect(),
        );
        let point = history_point(&snapshot);
        push_history(&mut self.history, self.history_capacity, point.clone());
        self.pending_records.push(RecorderRecord::History(point));

        self.last_snapshot = Arc::new(snapshot);
        if self.last_snapshot.step_kind == StepKind::Close {
            self.pending_records.push(RecorderRecord::Close {
                session_date: self.last_snapshot.session_date,
                snapshot: self.last_snapshot.clone(),
            });
            self.pending_records.push(RecorderRecord::DailyPnl {
                session_date: self.last_snapshot.session_date,
                buckets: self.last_snapshot.pnl.portfolio.today,
            });
        }
        Ok(self.last_snapshot.clone())
    }

    /// Points with `seq` greater than `since`, in ascending `seq` order, or every
    /// point currently held if `since` is `None`.
    pub fn history_since(&self, since: Option<u64>) -> Vec<HistoryPoint> {
        self.history
            .iter()
            .filter(|point| since.is_none_or(|s| point.seq > s))
            .cloned()
            .collect()
    }

    /// Applies one validated command, mutating engine state.
    fn apply(&mut self, command: EngineCommand) -> Result<EngineReply, EngineCommandError> {
        match command {
            EngineCommand::Pause => {
                self.clock.pause();
                self.push_control(ControlAction::Pause);
                Ok(EngineReply::Ack)
            }
            EngineCommand::Resume => {
                self.clock.resume();
                self.push_control(ControlAction::Resume);
                Ok(EngineReply::Ack)
            }
            EngineCommand::SetSpeed(speed) => {
                if !speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
                    return Err(EngineCommandError::InvalidSpeed(speed));
                }
                self.clock
                    .set_speed(speed)
                    .map_err(|_| EngineCommandError::InvalidSpeed(speed))?;
                self.push_control(ControlAction::SetSpeed { speed });
                Ok(EngineReply::Ack)
            }
            EngineCommand::AddPosition(spec) => self.add_position(&spec),
            EngineCommand::RemovePosition(id) => {
                let removed = self.book.remove(&id)?;
                self.pending_records
                    .push(RecorderRecord::Event(SessionEvent::PositionChange {
                        wall_time: Utc::now(),
                        sim_time: self.clock.now(),
                        kind: PositionChangeKind::Removed,
                        position_id: id,
                    }));
                self.pending_records
                    .push(RecorderRecord::CurrentPortfolio(self.active_positions()));
                Ok(EngineReply::Removed(Box::new(removed)))
            }
            EngineCommand::GetHistory { since } => {
                Ok(EngineReply::History(self.history_since(since)))
            }
            EngineCommand::GetSpecs => Ok(EngineReply::Specs(self.active_specs())),
            EngineCommand::LoadPortfolio { specs, source } => {
                let now = self.clock.now();
                // Bring the loaded book up to date with the splits already applied this run or
                // restored from the ledger: a version saved before a split comes back adjusted
                // once, one saved after it comes back unchanged.
                let specs = adjust_specs_for_ledger(specs, &self.applied_splits());
                let mut book = PositionBook::from_positions(resolve(&specs, &self.market, now)?)?;
                let settlements = book.settle_expired(now, &self.market);
                self.book = book;

                // Rebase P&L attribution: the same zero-length-`ClockStep` trick
                // `Engine::new` uses to seed the opening valuation, so the swap itself
                // contributes no P&L and the very next real tick diffs cleanly against
                // the newly loaded book.
                self.attributor = PnlAttributor::new(self.last_snapshot.session_date);
                let zero_step = ClockStep {
                    from: now,
                    to: now,
                    trading_years: 0.0,
                    calendar_years: 0.0,
                    session_date: self.last_snapshot.session_date,
                    kind: StepKind::Intraday,
                };
                self.attributor
                    .on_tick(&self.book.value_all(&self.market, now), &zero_step);

                let position_count = self.book.len();
                self.pending_records
                    .push(RecorderRecord::Event(SessionEvent::PortfolioLoad {
                        wall_time: Utc::now(),
                        sim_time: now,
                        source,
                        position_count,
                    }));
                for event in settlements {
                    self.pending_records
                        .push(RecorderRecord::Event(SessionEvent::Settlement(event)));
                }
                self.pending_records
                    .push(RecorderRecord::CurrentPortfolio(self.active_positions()));
                Ok(EngineReply::Loaded(position_count))
            }
        }
    }

    /// Resolves and adds one position, stamps it with the ledgered splits, records it.
    fn add_position(&mut self, spec: &PositionSpec) -> Result<EngineReply, EngineCommandError> {
        let now = self.clock.now();
        let resolved = resolve(std::slice::from_ref(spec), &self.market, now)?;
        // `resolve` returns exactly one position per spec on success; this
        // branch cannot happen in practice.
        let mut position = resolved.into_iter().next().ok_or_else(|| {
            EngineCommandError::Portfolio(PortfolioError::MissingField {
                id: spec.id.clone(),
                field: "spec",
            })
        })?;
        // A position added now is written in the market's current, post-split terms:
        // stamp it with every ledgered split on its symbol so a later reload of the book
        // does not adjust it again. It is not marked `adjusted`, since nothing was
        // changed.
        let splits = self.applied_splits();
        let symbol = position.instrument.underlying().clone();
        let stamps: Vec<String> = missing_splits(&splits, &symbol, &position.applied_actions)
            .map(|s| s.id.clone())
            .collect();
        position.applied_actions.extend(stamps);
        self.book.add(position)?;
        let rows = self.book.value_all(&self.market, now);
        let row = rows.into_iter().find(|r| r.id == spec.id).ok_or_else(|| {
            EngineCommandError::Portfolio(PortfolioError::UnknownPosition(spec.id.clone()))
        })?;
        self.pending_records
            .push(RecorderRecord::Event(SessionEvent::PositionChange {
                wall_time: Utc::now(),
                sim_time: now,
                kind: PositionChangeKind::Added,
                position_id: spec.id.clone(),
            }));
        self.pending_records
            .push(RecorderRecord::CurrentPortfolio(self.active_positions()));
        Ok(EngineReply::Added(Box::new(row)))
    }

    /// Pushes a [`SessionEvent::Control`] record for the recorder.
    fn push_control(&mut self, action: ControlAction) {
        self.pending_records
            .push(RecorderRecord::Event(SessionEvent::Control {
                wall_time: Utc::now(),
                sim_time: self.clock.now(),
                action,
            }));
    }

    /// Consumes the engine and spawns its tick loop on the current Tokio runtime.
    ///
    /// The loop uses `tokio::time::interval(tick_interval)` with
    /// `MissedTickBehavior::Skip`, and `select!`s over the interval, the command
    /// channel and `shutdown`. It exits only when `shutdown` is cancelled — a data
    /// error never stops it, per [`Engine::tick`]'s recovery rule.
    ///
    /// Inter-tick elapsed time is measured with `tokio::time::Instant`, not
    /// `std::time::Instant`, so that a paused Tokio clock (`#[tokio::test(start_paused =
    /// true)]` plus `tokio::time::advance`) drives `real_elapsed` correctly in tests.
    /// `storage` is used only to start the recorder task (see [`crate::recorder`]) —
    /// the tick-loop task spawned here never touches it itself; every record it
    /// produces is handed to the recorder over a channel with a non-blocking
    /// `try_send`. Returns the handle callers use to talk to the engine, and the tick
    /// loop's own `JoinHandle`, which resolves once `shutdown` is cancelled and the
    /// recorder has finished draining, flushing and writing the final `meta.json` —
    /// awaiting it is what makes "stop the engine, drain, flush, write the final
    /// meta.json, then exit" actually happen in order.
    pub fn spawn(
        mut self,
        tick_interval: Duration,
        storage: StorageRoot,
        shutdown: CancellationToken,
    ) -> (EngineHandle, tokio::task::JoinHandle<()>)
    where
        F: Send + 'static,
        C: Send + 'static,
    {
        let (snapshot_tx, snapshot_rx) = watch::channel(self.last_snapshot.clone());
        let (alert_tx, _alert_rx) = broadcast::channel(256);
        let (request_tx, mut request_rx) = mpsc::channel::<EngineRequest>(32);
        let alert_tx_for_task = alert_tx.clone();

        let (recorder_tx, recorder_rx) = mpsc::channel(recorder::RECORDER_CHANNEL_CAPACITY);
        let dropped_records = Arc::new(AtomicU64::new(0));
        let recorder_join = tokio::spawn(recorder::run(
            recorder_rx,
            storage,
            self.session_meta.clone(),
            dropped_records.clone(),
        ));
        let dropped_records_for_task = dropped_records.clone();
        let session_id: Arc<str> = Arc::from(self.session_id.as_str());

        let engine_join = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_interval);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut last_tick_at = TokioInstant::now();
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        let now = TokioInstant::now();
                        let elapsed = now.duration_since(last_tick_at);
                        last_tick_at = now;
                        if let Ok(snapshot) = self.tick(elapsed) {
                            let _ = snapshot_tx.send(snapshot);
                        }
                        for event in std::mem::take(&mut self.new_alerts) {
                            let _ = alert_tx_for_task.send(event);
                        }
                        send_pending_records(&mut self.pending_records, &recorder_tx, &dropped_records_for_task);
                    }
                    Some(request) = request_rx.recv() => {
                        let result = self.apply(request.command);
                        send_pending_records(&mut self.pending_records, &recorder_tx, &dropped_records_for_task);
                        let _ = request.reply.send(result);
                    }
                }
            }
            drop(recorder_tx);
            let _ = recorder_join.await;
        });

        (
            EngineHandle::new(
                snapshot_rx,
                alert_tx,
                request_tx,
                dropped_records,
                session_id,
            ),
            engine_join,
        )
    }
}

/// Drains `pending_records`, handing each to the recorder with a non-blocking
/// `try_send`. A full channel counts as a dropped record and is otherwise ignored —
/// this is the one place the engine task ever touches the recorder, and it never waits.
fn send_pending_records(
    pending_records: &mut Vec<RecorderRecord>,
    recorder_tx: &mpsc::Sender<RecorderRecord>,
    dropped_records: &Arc<AtomicU64>,
) {
    for record in pending_records.drain(..) {
        if recorder_tx.try_send(record).is_err() {
            dropped_records.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Each symbol's current spot, for tracking a session's opening prices.
fn session_open_spots(market: &MarketState) -> BTreeMap<Symbol, f64> {
    market
        .iter()
        .map(|(symbol, state)| (symbol.clone(), state.spot()))
        .collect()
}

/// Builds a `RiskSnapshot` from its ingredients. A free function rather than a `&self`
/// method so `Engine::new` can call it before `Self` exists, with exactly the same
/// logic `Engine::tick` uses afterwards.
#[allow(clippy::too_many_arguments)]
fn build_snapshot(
    seq: u64,
    wall_time: chrono::DateTime<Utc>,
    step: &ClockStep,
    market: &MarketState,
    open_spots: &BTreeMap<Symbol, f64>,
    rows: Vec<portfolio::PositionRisk>,
    summary: risk::RiskSummary,
    pnl: risk::PnlReport,
    active_alerts: Vec<LimitStatus>,
    limits: Vec<LimitStatus>,
    alert_history: Vec<AlertEvent>,
    clock_speed: f64,
    paused: bool,
    feed_status: FeedStatus,
    compute_micros: u64,
    var: risk::VarReport,
    corporate_actions: Vec<CorporateActionEvent>,
) -> RiskSnapshot {
    let market_snapshot = market
        .iter()
        .map(|(symbol, state)| {
            let spot = state.spot();
            let open = open_spots.get(symbol).copied().unwrap_or(spot);
            let change_since_open = if open == 0.0 {
                0.0
            } else {
                (spot - open) / open
            };
            (
                symbol.clone(),
                SymbolSnapshot {
                    symbol: symbol.clone(),
                    spot,
                    vol: state.vol(),
                    change_since_open,
                },
            )
        })
        .collect();

    RiskSnapshot {
        seq,
        wall_time,
        sim_time: step.to,
        session_date: step.session_date,
        step_kind: step.kind,
        market: market_snapshot,
        positions: rows,
        summary,
        pnl,
        var,
        active_alerts,
        limits,
        alert_history,
        clock_speed,
        paused,
        feed_status,
        compute_micros,
        corporate_actions,
    }
}

/// Projects a [`RiskSnapshot`] down to the [`HistoryPoint`] kept in the history ring
/// buffer.
fn history_point(snapshot: &RiskSnapshot) -> HistoryPoint {
    let worst_limit_utilisation = snapshot
        .limits
        .iter()
        .map(|status| status.utilisation)
        .fold(0.0_f64, f64::max);
    HistoryPoint {
        seq: snapshot.seq,
        sim_time: snapshot.sim_time,
        dollar_delta: snapshot.summary.totals.dollar_delta,
        dollar_gamma_1pct: snapshot.summary.totals.dollar_gamma_1pct,
        vega: snapshot.summary.totals.vega,
        theta: snapshot.summary.totals.theta,
        gross_exposure: snapshot.summary.totals.gross_exposure,
        market_value: snapshot.summary.totals.market_value,
        pnl_today: snapshot.pnl.portfolio.today,
        worst_limit_utilisation,
        var_99: Some(snapshot.var.headline().var_99),
    }
}

/// Appends `point` to `history`, evicting the oldest point first once `history` is at
/// `capacity`. A `capacity` of `0` makes this a no-op, rather than panicking.
fn push_history(history: &mut VecDeque<HistoryPoint>, capacity: usize, point: HistoryPoint) {
    if capacity == 0 {
        return;
    }
    if history.len() >= capacity {
        history.pop_front();
    }
    history.push_back(point);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use chrono::TimeZone;

    use super::*;

    fn control_record(speed: f64) -> RecorderRecord {
        let now = Utc.with_ymd_and_hms(2026, 9, 26, 4, 15, 0).unwrap();
        RecorderRecord::Event(SessionEvent::Control {
            wall_time: now,
            sim_time: now,
            action: ControlAction::SetSpeed { speed },
        })
    }

    #[test]
    fn a_full_channel_drops_the_records_the_receiver_has_no_room_for() {
        let (tx, mut rx) = mpsc::channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut pending: Vec<RecorderRecord> =
            (0..5).map(|i| control_record(f64::from(i))).collect();

        send_pending_records(&mut pending, &tx, &dropped);

        assert!(pending.is_empty(), "every record must be drained");
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            3,
            "5 records into a channel with room for 2 leaves 3 dropped"
        );
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
        assert!(
            rx.try_recv().is_err(),
            "only the first 2 records should have made it onto the channel"
        );
    }
}
