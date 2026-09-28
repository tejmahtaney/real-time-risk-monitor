//! `Engine`: owns all mutable state, ticks the feed, values the book, publishes
//! snapshots. Nothing else touches the book, the feed or the clock, so there are no
//! locks on the hot path.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use domain::{Clock, ClockStep, MarketState, SimClock, StepKind, Symbol, market_date};
use marketdata::{PriceFeed, SeedData};
use portfolio::{PortfolioError, PositionBook, PositionSpec, resolve};
use risk::{AlertEvent, LimitMonitor, LimitStatus, LimitsConfig, PnlAttributor, aggregate};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{Instant as TokioInstant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
use crate::error::EngineError;
use crate::handle::EngineHandle;
use crate::snapshot::{FeedStatus, RiskSnapshot, SymbolSnapshot};

/// Lower bound for [`EngineCommand::SetSpeed`], inclusive.
const MIN_SPEED: f64 = 1.0;
/// Upper bound for [`EngineCommand::SetSpeed`], inclusive.
const MAX_SPEED: f64 = 3600.0;

/// Owns the position book, price feed, clock, market state, P&L attributor and limit
/// monitor. Generic over the feed so tests can plug in a fake that fails on command;
/// production code instantiates `Engine<marketdata::GbmSimulator>` exactly once.
pub struct Engine<F: PriceFeed> {
    clock: SimClock,
    feed: F,
    market: MarketState,
    book: PositionBook,
    attributor: PnlAttributor,
    monitor: LimitMonitor,
    /// Each symbol's spot at the start of the current session, for `change_since_open`.
    open_spots: BTreeMap<Symbol, f64>,
    seq: u64,
    last_snapshot: Arc<RiskSnapshot>,
    /// Alert events from the most recent call to `tick`, drained by `spawn`'s loop onto
    /// the broadcast channel.
    new_alerts: Vec<AlertEvent>,
}

impl<F: PriceFeed> Engine<F> {
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
    /// events; production code passes a real clock, tests a fixed one.
    ///
    /// # Errors
    ///
    /// [`EngineError::Calibration`] if the seed cannot build a market state,
    /// [`EngineError::Portfolio`] if the starting positions cannot be resolved or added
    /// to the book.
    pub fn new(
        clock: SimClock,
        seed: &SeedData,
        feed: F,
        specs: &[PositionSpec],
        limits: LimitsConfig,
        wall_clock: Box<dyn Clock>,
    ) -> Result<Self, EngineError> {
        let started = Instant::now();
        let start = clock.now();
        let session_date = market_date(start);

        let market = seed.to_market_state(start)?;
        let positions = resolve(specs, &market, start)?;
        let book = PositionBook::from_positions(positions)?;

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
        let _events = monitor.evaluate(&summary, start);

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
            alert_history,
            clock.speed(),
            clock.is_paused(),
            FeedStatus::Ok,
            compute_micros,
        );

        Ok(Self {
            clock,
            feed,
            market,
            book,
            attributor,
            monitor,
            open_spots,
            seq: 0,
            last_snapshot: Arc::new(snapshot),
            new_alerts: Vec::new(),
        })
    }

    /// One full cycle: advance the clock, step the feed, settle expiries, value
    /// positions, aggregate, attribute P&L, evaluate limits, build and publish a
    /// snapshot.
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

        if step.kind == StepKind::Overnight {
            self.open_spots = session_open_spots(&self.market);
        }

        self.book.settle_expired(step.to, &self.market);
        let rows = self.book.value_all(&self.market, step.to);
        self.attributor.on_tick(&rows, &step);
        let summary = aggregate(&rows);
        let events = self.monitor.evaluate(&summary, step.to);
        self.new_alerts.extend(events);

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
            alert_history,
            self.clock.speed(),
            self.clock.is_paused(),
            feed_status,
            compute_micros,
        );
        self.last_snapshot = Arc::new(snapshot);
        Ok(self.last_snapshot.clone())
    }

    /// Applies one validated command, mutating engine state.
    fn apply(&mut self, command: EngineCommand) -> Result<EngineReply, EngineCommandError> {
        match command {
            EngineCommand::Pause => {
                self.clock.pause();
                Ok(EngineReply::Ack)
            }
            EngineCommand::Resume => {
                self.clock.resume();
                Ok(EngineReply::Ack)
            }
            EngineCommand::SetSpeed(speed) => {
                if !speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
                    return Err(EngineCommandError::InvalidSpeed(speed));
                }
                self.clock
                    .set_speed(speed)
                    .map_err(|_| EngineCommandError::InvalidSpeed(speed))?;
                Ok(EngineReply::Ack)
            }
            EngineCommand::AddPosition(spec) => {
                let now = self.clock.now();
                let resolved = resolve(std::slice::from_ref(&spec), &self.market, now)?;
                // `resolve` returns exactly one position per spec on success; this
                // branch cannot happen in practice.
                let position = resolved.into_iter().next().ok_or_else(|| {
                    EngineCommandError::Portfolio(PortfolioError::MissingField {
                        id: spec.id.clone(),
                        field: "spec",
                    })
                })?;
                self.book.add(position)?;
                let rows = self.book.value_all(&self.market, now);
                let row = rows.into_iter().find(|r| r.id == spec.id).ok_or_else(|| {
                    EngineCommandError::Portfolio(PortfolioError::UnknownPosition(spec.id.clone()))
                })?;
                Ok(EngineReply::Added(Box::new(row)))
            }
            EngineCommand::RemovePosition(id) => {
                let removed = self.book.remove(&id)?;
                Ok(EngineReply::Removed(Box::new(removed)))
            }
        }
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
    pub fn spawn(mut self, tick_interval: Duration, shutdown: CancellationToken) -> EngineHandle
    where
        F: Send + 'static,
    {
        let (snapshot_tx, snapshot_rx) = watch::channel(self.last_snapshot.clone());
        let (alert_tx, _alert_rx) = broadcast::channel(256);
        let (request_tx, mut request_rx) = mpsc::channel::<EngineRequest>(32);
        let alert_tx_for_task = alert_tx.clone();

        tokio::spawn(async move {
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
                    }
                    Some(request) = request_rx.recv() => {
                        let result = self.apply(request.command);
                        let _ = request.reply.send(result);
                    }
                }
            }
        });

        EngineHandle::new(snapshot_rx, alert_tx, request_tx)
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
    alert_history: Vec<AlertEvent>,
    clock_speed: f64,
    paused: bool,
    feed_status: FeedStatus,
    compute_micros: u64,
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
        market: market_snapshot,
        positions: rows,
        summary,
        pnl,
        active_alerts,
        alert_history,
        clock_speed,
        paused,
        feed_status,
        compute_micros,
    }
}
