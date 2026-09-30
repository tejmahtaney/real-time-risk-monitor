//! The backtest manager: creates runs, holds the unfinished ones in memory, and reads saved ones.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use risk::{BacktestSummary, DailyRow, summarise};
use storage::remove_dir_confined;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::error::BacktestError;
use super::recorder::{self, BacktestRecord};
use super::runner::{ControlCommand, Job, JobMessage, Progress, Shared};
use super::saved::{self, SavedRun};
use super::setup::{
    BacktestConfig, CreateOptions, MAX_SPEED, MIN_SPEED, Prepared, RunInput, prepare,
};
use super::types::{
    BACKTESTS_FOLDER, BacktestListItem, BacktestResults, BacktestRunState, BacktestSettings,
    BacktestStatus,
};
use crate::snapshot::RiskSnapshot;
use risk::AlertEvent;

/// One run held in memory.
struct Run {
    shared: Arc<Shared>,
    messages: std::sync::mpsc::Sender<JobMessage>,
    job: Mutex<Option<JoinHandle<()>>>,
    recorder: Mutex<Option<JoinHandle<()>>>,
}

impl Run {
    /// Stops the replay job and waits for it and for the recorder to finish, so every file the run
    /// will ever write has been written when this returns.
    async fn shut_down(&self) {
        let _ = self.messages.send(JobMessage::Stop);
        let job = lock(&self.job).take();
        if let Some(job) = job {
            let _ = job.await;
        }
        let recorder = lock(&self.recorder).take();
        if let Some(recorder) = recorder {
            let _ = recorder.await;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Holds a WebSocket client's place on a run. While one exists the run counts as in use and is not
/// freed for being idle. Dropping it starts the idle clock again.
pub struct ClientGuard {
    shared: Arc<Shared>,
}

impl ClientGuard {
    fn new(shared: Arc<Shared>) -> Self {
        shared.clients.fetch_add(1, Ordering::Relaxed);
        shared.touch();
        Self { shared }
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.shared.clients.fetch_sub(1, Ordering::Relaxed);
        self.shared.touch();
    }
}

/// What a WebSocket handler needs to stream a run: the receivers and the guard that keeps the run
/// alive while it does.
pub struct BacktestSubscription {
    /// The latest snapshot, updated once per replayed date.
    pub snapshots: watch::Receiver<Arc<RiskSnapshot>>,
    /// Limit alerts as they happen.
    pub alerts: broadcast::Receiver<AlertEvent>,
    /// The run's speed in dates per second when subscribed.
    pub speed: f64,
    /// Keep this alive for as long as the client is connected.
    pub guard: ClientGuard,
}

/// A slot for a run being created, so two simultaneous creates cannot both take the last place.
struct Reservation<'a> {
    pending: &'a AtomicUsize,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Creates, controls and reads backtest runs.
///
/// At most [`BacktestConfig::max_running`] runs may be in memory and unfinished. A run's in-memory
/// state is freed by [`sweep`](Self::sweep) once nothing has used it for
/// [`BacktestConfig::idle_after`]; its files stay, and its results are then read from them.
pub struct BacktestManager {
    config: BacktestConfig,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    pending: AtomicUsize,
}

impl BacktestManager {
    /// A manager with no runs in memory.
    pub fn new(config: BacktestConfig) -> Self {
        Self {
            config,
            runs: Mutex::new(HashMap::new()),
            pending: AtomicUsize::new(0),
        }
    }

    fn running_count(runs: &HashMap<String, Arc<Run>>) -> usize {
        runs.values()
            .filter(|run| run.shared.progress().state.is_running())
            .count()
    }

    fn reserve(&self) -> Result<Reservation<'_>, BacktestError> {
        let runs = lock(&self.runs);
        let running = Self::running_count(&runs) + self.pending.load(Ordering::Relaxed);
        if running >= self.config.max_running {
            return Err(BacktestError::TooManyRunning {
                running,
                limit: self.config.max_running,
            });
        }
        self.pending.fetch_add(1, Ordering::Relaxed);
        Ok(Reservation {
            pending: &self.pending,
        })
    }

    fn find(&self, id: &str) -> Option<Arc<Run>> {
        lock(&self.runs).get(id).cloned()
    }

    fn require(&self, id: &str) -> Result<Arc<Run>, BacktestError> {
        let run = self.find(id).ok_or_else(|| {
            BacktestError::NotFound(format!(
                "backtest {id} is not in memory (only its saved files remain)"
            ))
        })?;
        run.shared.touch();
        Ok(run)
    }

    /// Creates a run from an upload or the archive, saves its starting files and leaves it paused
    /// at the start date.
    ///
    /// # Errors
    ///
    /// [`BacktestError::TooManyRunning`] at the limit, [`BacktestError::Invalid`] for a speed
    /// outside 1 to 50, and the data, portfolio and storage errors for input that cannot be used.
    pub async fn create(
        &self,
        input: RunInput,
        options: CreateOptions,
    ) -> Result<BacktestStatus, BacktestError> {
        check_speed(options.speed)?;
        let reservation = self.reserve()?;
        let config = self.config.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare(&config, input, options))
            .await
            .map_err(|e| BacktestError::Internal(format!("the setup task failed: {e}")))??;
        let Prepared {
            engine,
            table,
            settings,
            input,
        } = prepared;
        let id = settings.run_id.clone();

        let (records, records_rx) = mpsc::unbounded_channel();
        let recorder = tokio::spawn(recorder::run(
            records_rx,
            self.config.storage.clone(),
            id.clone(),
        ));
        let (ack, acked) = oneshot::channel();
        let sent = records.send(BacktestRecord::Init {
            input,
            settings: Box::new(settings.clone()),
            config_paths: self.config.config_paths.clone(),
            ack,
        });
        let written = match (sent, acked.await) {
            (Ok(()), Ok(result)) => result,
            _ => Err("the recorder stopped before saving the run".to_owned()),
        };
        if let Err(message) = written {
            drop(records);
            let _ = recorder.await;
            return Err(BacktestError::Internal(format!(
                "cannot save the run's files: {message}"
            )));
        }

        let (snapshot_tx, _) = watch::channel(engine.latest_snapshot());
        let (alert_tx, _) = broadcast::channel(256);
        let shared = Arc::new(Shared {
            id: id.clone(),
            progress: Mutex::new(Progress {
                state: BacktestRunState::Paused,
                speed: settings.speed,
                rows: Vec::new(),
                events: Vec::new(),
                summary: None,
            }),
            settings,
            snapshot_tx,
            alert_tx,
            clients: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
        });
        let (messages, messages_rx) = std::sync::mpsc::channel();
        let job = Job {
            engine,
            table,
            shared: Arc::clone(&shared),
            records,
            messages: messages_rx,
        };
        let job = tokio::task::spawn_blocking(move || job.run());
        let run = Arc::new(Run {
            shared,
            messages,
            job: Mutex::new(Some(job)),
            recorder: Mutex::new(Some(recorder)),
        });
        let status = status_of(&run.shared, true);
        {
            let mut runs = lock(&self.runs);
            runs.insert(id, run);
            // Inside the lock, so a concurrent create sees the run and not a phantom reservation.
            drop(reservation);
        }
        Ok(status)
    }

    /// Applies a control command to a run in memory and returns its status.
    ///
    /// # Errors
    ///
    /// [`BacktestError::NotFound`] when the run is not in memory, [`BacktestError::Invalid`] for a
    /// finished run or a speed outside 1 to 50 dates per second.
    pub fn control(
        &self,
        id: &str,
        command: ControlCommand,
    ) -> Result<BacktestStatus, BacktestError> {
        let run = self.require(id)?;
        if let ControlCommand::SetSpeed(speed) = command {
            check_speed(speed)?;
        }
        {
            let mut progress = run.shared.progress();
            if !progress.state.is_running() {
                return Err(BacktestError::Invalid(format!(
                    "backtest {id} has finished, so it cannot take a command"
                )));
            }
            match command {
                ControlCommand::Play => progress.state = BacktestRunState::Playing,
                ControlCommand::RunToEnd => progress.state = BacktestRunState::RunningToEnd,
                ControlCommand::Pause | ControlCommand::Step => {
                    progress.state = BacktestRunState::Paused;
                }
                ControlCommand::SetSpeed(speed) => progress.speed = speed,
            }
        }
        run.messages
            .send(JobMessage::Control(command))
            .map_err(|_| BacktestError::Invalid(format!("backtest {id} has already stopped")))?;
        Ok(status_of(&run.shared, true))
    }

    /// The status of a run, from memory if it is there, else from its files.
    ///
    /// # Errors
    ///
    /// [`BacktestError::NotFound`] for an unknown run.
    pub fn status(&self, id: &str) -> Result<BacktestStatus, BacktestError> {
        if let Some(run) = self.find(id) {
            run.shared.touch();
            return Ok(status_of(&run.shared, true));
        }
        let dir = saved::run_dir(&self.config.storage, id)?;
        let settings = saved::read_settings(&dir)
            .map_err(|_| BacktestError::NotFound(format!("no backtest run {id}")))?;
        let summary = saved::read_summary(&dir)?;
        let rows = saved::read_rows(&dir)?;
        Ok(saved_status(&settings, &rows, summary.as_ref()))
    }

    /// Everything a run has produced: its rows, summary and events. For a run in memory these are
    /// what it has so far; for a saved run they are read from its files.
    ///
    /// # Errors
    ///
    /// [`BacktestError::NotFound`] for an unknown run.
    pub fn results(&self, id: &str) -> Result<BacktestResults, BacktestError> {
        if let Some(run) = self.find(id) {
            run.shared.touch();
            let (rows, events, summary) = {
                let progress = run.shared.progress();
                (
                    progress.rows.clone(),
                    progress.events.clone(),
                    progress.summary.clone(),
                )
            };
            let status = status_of(&run.shared, true);
            return Ok(assemble(
                status,
                run.shared.settings.clone(),
                rows,
                events,
                summary,
            ));
        }
        let SavedRun {
            settings,
            rows,
            events,
            summary,
        } = saved::read_saved(&self.config.storage, id)?;
        let status = saved_status(&settings, &rows, summary.as_ref());
        Ok(assemble(status, settings, rows, events, summary))
    }

    /// Every saved run, newest first, with runs in memory shown in their live state.
    ///
    /// # Errors
    ///
    /// [`BacktestError::Storage`] when the folder cannot be read.
    pub fn list(&self) -> Result<Vec<BacktestListItem>, BacktestError> {
        let mut items = Vec::new();
        for id in saved::run_ids(&self.config.storage)? {
            let dir = saved::run_dir(&self.config.storage, &id)?;
            let Ok(settings) = saved::read_settings(&dir) else {
                tracing::warn!(%id, "skipping a backtest folder with no readable settings.json");
                continue;
            };
            let live = self.find(&id);
            let (state, days, total_pnl) = if let Some(run) = &live {
                let progress = run.shared.progress();
                (
                    progress.state,
                    progress.rows.len(),
                    progress.rows.last().map(|r| r.cumulative_pnl),
                )
            } else if let Some(summary) = saved::read_summary(&dir)? {
                (
                    BacktestRunState::Finished,
                    summary.days,
                    Some(summary.total.actual),
                )
            } else {
                let rows = saved::read_rows(&dir)?;
                (
                    BacktestRunState::Unfinished,
                    rows.len(),
                    rows.last().map(|r| r.cumulative_pnl),
                )
            };
            items.push(BacktestListItem {
                id,
                state,
                source: settings.source,
                start_date: settings.start_date,
                last_date: settings.last_date,
                days,
                total_pnl,
                created: settings.created,
            });
        }
        Ok(items)
    }

    /// Stops a run and frees its memory. Its files stay.
    ///
    /// # Errors
    ///
    /// [`BacktestError::NotFound`] when the run is not in memory.
    pub async fn stop(&self, id: &str) -> Result<(), BacktestError> {
        let run = lock(&self.runs)
            .remove(id)
            .ok_or_else(|| BacktestError::NotFound(format!("backtest {id} is not in memory")))?;
        run.shut_down().await;
        Ok(())
    }

    /// Deletes a run's folder from disk. The one delete in the app, run only on a user's request.
    ///
    /// `confirm` must equal `id`. That is checked **first**, before anything is looked up or
    /// touched, so a stray request cannot delete anything. A run in memory is stopped and its
    /// recorder awaited before the folder goes, so nothing writes into it afterwards.
    ///
    /// # Errors
    ///
    /// [`BacktestError::Invalid`] when `confirm` does not match, [`BacktestError::Storage`] for a
    /// path that is unsafe or a folder that cannot be removed, and [`BacktestError::NotFound`]
    /// when there is no such folder.
    pub async fn delete_files(&self, id: &str, confirm: &str) -> Result<(), BacktestError> {
        if confirm != id {
            return Err(BacktestError::Invalid(
                "to delete a backtest, send {\"confirm\": \"<the run id>\"}".to_owned(),
            ));
        }
        storage::check_part(id)?;
        let live = lock(&self.runs).remove(id);
        if let Some(run) = live {
            run.shut_down().await;
        }
        remove_dir_confined(&self.config.storage, BACKTESTS_FOLDER, id).map_err(
            |error| match &error {
                storage::StorageError::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    BacktestError::NotFound(format!("no backtest run {id}"))
                }
                _ => BacktestError::Storage(error),
            },
        )
    }

    /// Subscribes to a run in memory for streaming. The returned guard keeps the run from being
    /// freed as idle while it is held.
    ///
    /// # Errors
    ///
    /// [`BacktestError::NotFound`] when the run is not in memory.
    pub fn subscribe(&self, id: &str) -> Result<BacktestSubscription, BacktestError> {
        let run = self.require(id)?;
        let guard = ClientGuard::new(Arc::clone(&run.shared));
        let speed = run.shared.progress().speed;
        Ok(BacktestSubscription {
            snapshots: run.shared.snapshot_tx.subscribe(),
            alerts: run.shared.alert_tx.subscribe(),
            speed,
            guard,
        })
    }

    /// Frees the runs nobody is using and returns their ids.
    ///
    /// A run is freed only when **all** of these hold, and never otherwise:
    ///
    /// - it is not playing and not running to the end,
    /// - no WebSocket client is connected to it, and
    /// - nothing has used it through the API for at least [`BacktestConfig::idle_after`] as of `now`.
    ///
    /// A finished run and a paused one are both freed on the same terms. Freeing stops the replay
    /// job, waits for the recorder, and drops the memory; the files stay.
    pub async fn sweep(&self, now: Instant) -> Vec<String> {
        let idle: Vec<(String, Arc<Run>)> = {
            let runs = lock(&self.runs);
            runs.iter()
                .filter(|(_, run)| {
                    let state = run.shared.progress().state;
                    !matches!(
                        state,
                        BacktestRunState::Playing | BacktestRunState::RunningToEnd
                    ) && run.shared.client_count() == 0
                        && run.shared.idle_for(now) >= self.config.idle_after
                })
                .map(|(id, run)| (id.clone(), Arc::clone(run)))
                .collect()
        };
        let mut freed = Vec::new();
        for (id, run) in idle {
            let removed = {
                let mut runs = lock(&self.runs);
                // Look again under the lock: a client may have connected since the scan.
                let still_idle = run.shared.client_count() == 0
                    && !matches!(
                        run.shared.progress().state,
                        BacktestRunState::Playing | BacktestRunState::RunningToEnd
                    )
                    && run.shared.idle_for(now) >= self.config.idle_after;
                if still_idle { runs.remove(&id) } else { None }
            };
            if let Some(run) = removed {
                tracing::info!(%id, "freeing an idle backtest from memory; its files stay");
                run.shut_down().await;
                freed.push(id);
            }
        }
        freed
    }

    /// Stops every run in memory and waits for their recorders. Call once at shutdown.
    pub async fn shutdown(&self) {
        let runs: Vec<Arc<Run>> = lock(&self.runs).drain().map(|(_, run)| run).collect();
        for run in runs {
            run.shut_down().await;
        }
    }

    /// Spawns a task that calls [`sweep`](Self::sweep) every `every` until `shutdown` is cancelled.
    pub fn spawn_janitor(
        self: &Arc<Self>,
        every: Duration,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        manager.sweep(Instant::now()).await;
                    }
                }
            }
        })
    }
}

fn check_speed(speed: f64) -> Result<(), BacktestError> {
    if speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&speed) {
        Ok(())
    } else {
        Err(BacktestError::Invalid(format!(
            "speed must be between {MIN_SPEED} and {MAX_SPEED} dates per second, got {speed}"
        )))
    }
}

fn status_of(shared: &Shared, in_memory: bool) -> BacktestStatus {
    let progress = shared.progress();
    let settings = &shared.settings;
    let done = progress.rows.len();
    BacktestStatus {
        id: shared.id.clone(),
        state: progress.state,
        in_memory,
        speed: progress.speed,
        roll_expired: settings.roll_expired,
        source: settings.source.clone(),
        start_date: settings.start_date,
        last_date: settings.last_date,
        current_date: progress.rows.last().map_or(settings.start_date, |r| r.date),
        steps_done: done,
        steps_total: settings.replay_dates,
        progress: fraction(done, settings.replay_dates),
    }
}

fn saved_status(
    settings: &BacktestSettings,
    rows: &[DailyRow],
    summary: Option<&BacktestSummary>,
) -> BacktestStatus {
    BacktestStatus {
        id: settings.run_id.clone(),
        state: if summary.is_some() {
            BacktestRunState::Finished
        } else {
            BacktestRunState::Unfinished
        },
        in_memory: false,
        speed: settings.speed,
        roll_expired: settings.roll_expired,
        source: settings.source.clone(),
        start_date: settings.start_date,
        last_date: settings.last_date,
        current_date: rows.last().map_or(settings.start_date, |r| r.date),
        steps_done: rows.len(),
        steps_total: settings.replay_dates,
        progress: fraction(rows.len(), settings.replay_dates),
    }
}

fn fraction(done: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).min(1.0)
    }
}

fn assemble(
    status: BacktestStatus,
    settings: BacktestSettings,
    rows: Vec<DailyRow>,
    events: Vec<super::types::BacktestEvent>,
    summary: Option<BacktestSummary>,
) -> BacktestResults {
    let partial = summary.is_none();
    let summary = summary.unwrap_or_else(|| summarise(&rows));
    BacktestResults {
        status,
        settings,
        rows,
        summary,
        partial,
        events,
    }
}
