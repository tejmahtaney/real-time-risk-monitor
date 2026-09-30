//! The backtester (Part 20): replays historical closes through the same [`Engine`](crate::Engine)
//! the live app uses, with a [`ReplayClock`](domain::ReplayClock) and a
//! [`ReplayFeed`](marketdata::ReplayFeed) in place of the simulator.
//!
//! - `setup`: reads an upload or the archive and builds the engine at the first date after the
//!   20-date warm-up.
//! - `runner`: the replay job, on the blocking pool, and the shared state it publishes to.
//! - `recorder`: the only writer of `backtests/<run_id>/`.
//! - `saved`: reads finished and unfinished runs back from disk.
//! - `manager`: creates runs, applies the limits, and frees idle ones.
//! - `roll`: replacing expired options so the book lasts the whole replay.

mod error;
mod manager;
mod recorder;
mod roll;
mod runner;
mod saved;
mod setup;
mod types;

pub use error::BacktestError;
pub use manager::{BacktestManager, BacktestSubscription, ClientGuard};
pub use roll::{BacktestRollEvent, RollPolicy, SpecRoller};
pub use runner::ControlCommand;
pub use setup::{BacktestConfig, CreateOptions, DEFAULT_SPEED, MAX_SPEED, MIN_SPEED, RunInput};
pub use types::{
    BACKTESTS_FOLDER, BacktestEvent, BacktestListItem, BacktestResults, BacktestRunState,
    BacktestSettings, BacktestSource, BacktestSplitHandling, BacktestSplitNote, BacktestStatus,
    DAILY_FILE, EVENTS_FILE, INPUT_FILE, SETTINGS_FILE, SUMMARY_FILE,
};
