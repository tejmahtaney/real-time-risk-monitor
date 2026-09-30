//! The replay job: drives the engine through the dates and publishes what it produces.
//!
//! # Why a blocking task
//!
//! Replaying is CPU work with no waiting in it. `run_to_end` computes every remaining date back to
//! back, each with a Monte Carlo `VaR`. On a runtime worker that would starve every other task,
//! including the live engine's tick loop. The job is therefore one `spawn_blocking` task, on the
//! blocking pool and never on a worker. It owns the engine, so no lock is held while computing. It
//! sleeps between dates by waiting on its command channel with a timeout, which is also how a
//! command interrupts a sleep, and it looks at that channel between every two dates so `pause` and
//! `stop` stay responsive even in the middle of `run_to_end`.
//!
//! # No look-ahead
//!
//! Before ticking to date *k* the job gives the `VaR` calculator the correlation and returns up to
//! *k* and no later. The headline `VaR` computed at the close of *k−1* is what the row for *k*
//! carries as its start-of-day figure, so a price on or after *k* cannot change it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use domain::{EngineClock, ReplayClock};
use marketdata::{PriceTable, ReplayFeed};
use risk::{AlertEvent, BacktestSummary, DailyRow, LimitState, summarise};
use tokio::sync::{broadcast, mpsc, watch};

use super::recorder::BacktestRecord;
use super::types::{BacktestEvent, BacktestRunState, BacktestSettings};
use crate::engine::Engine;
use crate::recorder::{RecorderRecord, SessionEvent};
use crate::snapshot::RiskSnapshot;

/// A command for a running replay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlCommand {
    /// Step at the run's speed.
    Play,
    /// Stop stepping.
    Pause,
    /// Replay exactly one date, then stay paused.
    Step,
    /// Change the speed, in dates per second.
    SetSpeed(f64),
    /// Compute every remaining date without delay.
    RunToEnd,
}

/// What the job is told over its channel. `Stop` ends it without a summary.
#[derive(Debug, Clone, Copy)]
pub(crate) enum JobMessage {
    Control(ControlCommand),
    Stop,
}

/// What a run has produced so far, behind one lock.
#[derive(Debug)]
pub(crate) struct Progress {
    pub state: BacktestRunState,
    pub speed: f64,
    pub rows: Vec<DailyRow>,
    pub events: Vec<BacktestEvent>,
    pub summary: Option<BacktestSummary>,
}

/// State shared between the manager, the replay job and any connected clients.
pub(crate) struct Shared {
    pub id: String,
    pub settings: BacktestSettings,
    pub progress: Mutex<Progress>,
    pub snapshot_tx: watch::Sender<Arc<RiskSnapshot>>,
    pub alert_tx: broadcast::Sender<AlertEvent>,
    /// Connected WebSocket clients, kept by [`super::manager::ClientGuard`].
    pub clients: AtomicUsize,
    /// The last time anything used this run through the API.
    pub last_used: Mutex<Instant>,
}

impl Shared {
    pub(crate) fn progress(&self) -> MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    pub(crate) fn idle_for(&self, now: Instant) -> Duration {
        let last = *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        now.saturating_duration_since(last)
    }

    pub(crate) fn client_count(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Paused,
    Playing,
    ToEnd,
}

/// Everything the replay job owns.
pub(crate) struct Job {
    pub engine: Engine<ReplayFeed, ReplayClock>,
    pub table: Arc<PriceTable>,
    pub shared: Arc<Shared>,
    pub records: mpsc::UnboundedSender<BacktestRecord>,
    pub messages: Receiver<JobMessage>,
}

/// How a step ended.
enum Stepped {
    /// There is more to replay.
    More,
    /// That was the last date and the summary has been sent.
    Finished,
    /// The engine failed, which it is not meant to.
    Failed,
}

impl Job {
    /// Runs until the replay finishes, `Stop` arrives or the manager drops its sender.
    pub(crate) fn run(mut self) {
        let mut mode = Mode::Paused;
        let mut start_var = self.engine.latest_snapshot().var.headline().var_99;
        let mut cumulative = 0.0;
        self.emit_initial_events();

        loop {
            let message = match mode {
                Mode::Paused => match self.messages.recv() {
                    Ok(message) => Some(message),
                    Err(_) => return,
                },
                Mode::Playing => match self.messages.recv_timeout(self.step_interval()) {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                Mode::ToEnd => match self.messages.try_recv() {
                    Ok(message) => Some(message),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                },
            };

            // A timeout while playing, or an empty channel while running to the end, means "step
            // now". A command is applied first; `step` and `run_to_end` also step straight away.
            let mut step_now = message.is_none() && mode != Mode::Paused;
            if let Some(message) = message {
                match message {
                    JobMessage::Stop => return,
                    JobMessage::Control(ControlCommand::Play) => mode = Mode::Playing,
                    JobMessage::Control(ControlCommand::Pause) => mode = Mode::Paused,
                    JobMessage::Control(ControlCommand::RunToEnd) => {
                        mode = Mode::ToEnd;
                        step_now = true;
                    }
                    JobMessage::Control(ControlCommand::Step) => {
                        mode = Mode::Paused;
                        step_now = true;
                    }
                    JobMessage::Control(ControlCommand::SetSpeed(speed)) => {
                        if self.engine.clock_mut().set_speed(speed).is_ok() {
                            self.shared.progress().speed = speed;
                        }
                    }
                }
                self.set_state(mode);
            }
            if !step_now {
                continue;
            }

            match self.step(&mut start_var, &mut cumulative) {
                Stepped::More => {}
                Stepped::Finished => return,
                Stepped::Failed => {
                    self.shared.progress().state = BacktestRunState::Unfinished;
                    return;
                }
            }
        }
    }

    fn step_interval(&self) -> Duration {
        let speed = self.engine.clock().speed();
        Duration::from_secs_f64(1.0 / speed.max(f64::MIN_POSITIVE)).min(Duration::from_secs(3600))
    }

    fn set_state(&self, mode: Mode) {
        self.shared.progress().state = match mode {
            Mode::Paused => BacktestRunState::Paused,
            Mode::Playing => BacktestRunState::Playing,
            Mode::ToEnd => BacktestRunState::RunningToEnd,
        };
    }

    /// Settlements that happened while the engine was being built, before tick 0.
    fn emit_initial_events(&mut self) {
        let initial: Vec<BacktestEvent> = self
            .engine
            .initial_settlements()
            .iter()
            .cloned()
            .map(BacktestEvent::Settlement)
            .collect();
        // Whatever the constructor queued is not a result of the replay.
        self.engine.pending_records.clear();
        for event in initial {
            self.record_event(event);
        }
    }

    fn record_event(&self, event: BacktestEvent) {
        self.shared.progress().events.push(event.clone());
        let _ = self.records.send(BacktestRecord::Event(event));
    }

    /// One date: set the `VaR` window, tick, build the row, publish.
    fn step(&mut self, start_var: &mut f64, cumulative: &mut f64) -> Stepped {
        let next = self.engine.clock().index() + 1;
        let window = self
            .table
            .var_window(next, self.shared.settings.var_window_days);
        self.engine
            .set_var_window(window.correlation, window.returns, window.dates);
        let snapshot = match self.engine.tick(Duration::ZERO) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::error!(%error, "the replay engine failed");
                return Stepped::Failed;
            }
        };

        let breaches = snapshot
            .limits
            .iter()
            .filter(|status| status.state == LimitState::Breach)
            .count();
        let row = DailyRow::new(
            snapshot.session_date,
            snapshot.pnl.portfolio.today,
            *cumulative,
            *start_var,
            u32::try_from(breaches).unwrap_or(u32::MAX),
        );
        *cumulative = row.cumulative_pnl;
        *start_var = snapshot.var.headline().var_99;

        for record in std::mem::take(&mut self.engine.pending_records) {
            if let Some(event) = map_record(record) {
                self.record_event(event);
            }
        }
        for alert in std::mem::take(&mut self.engine.new_alerts) {
            let _ = self.shared.alert_tx.send(alert);
        }

        let finished = self.engine.clock().is_finished();
        let summary = {
            let mut progress = self.shared.progress();
            progress.rows.push(row.clone());
            if finished {
                let summary = summarise(&progress.rows);
                progress.summary = Some(summary.clone());
                progress.state = BacktestRunState::Finished;
                Some(summary)
            } else {
                None
            }
        };
        let _ = self.records.send(BacktestRecord::Daily(row));
        self.shared.snapshot_tx.send_replace(snapshot);
        match summary {
            Some(summary) => {
                let _ = self.records.send(BacktestRecord::Summary(summary));
                Stepped::Finished
            }
            None => Stepped::More,
        }
    }
}

/// The backtest event an engine record stands for, if it is one a backtest keeps.
fn map_record(record: RecorderRecord) -> Option<BacktestEvent> {
    match record {
        RecorderRecord::Event(SessionEvent::Alert(alert)) => Some(BacktestEvent::Alert(alert)),
        RecorderRecord::Event(SessionEvent::Settlement(event)) => {
            Some(BacktestEvent::Settlement(event))
        }
        RecorderRecord::Event(SessionEvent::CorporateAction(event)) => {
            Some(BacktestEvent::CorporateAction(event))
        }
        RecorderRecord::Roll(event) => Some(BacktestEvent::Roll(event)),
        // Ticks, closes, the live portfolio file, the live ledger and position changes are not
        // part of a backtest's results. Nothing here can reach `portfolios/`.
        RecorderRecord::History(_)
        | RecorderRecord::Event(_)
        | RecorderRecord::Close { .. }
        | RecorderRecord::DailyPnl { .. }
        | RecorderRecord::CurrentPortfolio(_)
        | RecorderRecord::AppliedAction(_) => None,
    }
}
