//! The wire and file types of a backtest run: settings, events, status and results.

use chrono::{DateTime, NaiveDate, Utc};
use domain::Symbol;
use portfolio::{PositionSpec, SettlementEvent};
use risk::{AlertEvent, BacktestSummary, DailyRow, VarMethod};
use serde::{Deserialize, Serialize};

use super::roll::BacktestRollEvent;
use crate::corporate::CorporateActionEvent;

/// `input.csv`: the prices the run replays.
pub const INPUT_FILE: &str = "input.csv";
/// `settings.json`: how the run was set up.
pub const SETTINGS_FILE: &str = "settings.json";
/// `daily.csv`: one row per replayed date, appended as the run goes.
pub const DAILY_FILE: &str = "daily.csv";
/// `events.jsonl`: breaches, rolls, settlements and corporate actions.
pub const EVENTS_FILE: &str = "events.jsonl";
/// `summary.json`: written when the run finishes. A run without it is unfinished.
pub const SUMMARY_FILE: &str = "summary.json";
/// The folder under the storage root that holds every run.
pub const BACKTESTS_FOLDER: &str = "backtests";

/// Where a run's prices came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum BacktestSource {
    /// An uploaded CSV, saved unchanged as `input.csv`.
    Upload {
        /// The file name the browser sent, if any.
        filename: Option<String>,
    },
    /// Every archived download from the local market-data archive, merged.
    Archive,
}

/// What was done about a price break that looks like a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BacktestSplitHandling {
    /// Earlier closes were divided by the ratio, so the series is continuous. The book is
    /// resolved on the adjusted prices, so nothing needs adjusting.
    BackAdjusted,
    /// A configured split matched the break. The prices stay as traded and the engine applied the
    /// split to the book on its date, exactly as in live mode.
    AppliedByEngine,
}

/// A price break the replay treated as a split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestSplitNote {
    /// The symbol.
    pub symbol: Symbol,
    /// The first date at the new price.
    pub date: NaiveDate,
    /// New shares per old share.
    pub ratio: f64,
    /// How it was handled.
    pub handling: BacktestSplitHandling,
}

/// `settings.json`: everything needed to understand, and to redo, a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestSettings {
    /// The run's id, in the same form as a session id.
    pub run_id: String,
    /// When the run was created.
    pub created: DateTime<Utc>,
    /// Where the prices came from.
    pub source: BacktestSource,
    /// The symbols replayed: the portfolio's underlyings.
    pub symbols: Vec<Symbol>,
    /// Symbols in the input that no position uses, ignored.
    pub ignored_symbols: Vec<Symbol>,
    /// The first date with a price for every symbol.
    pub first_date: NaiveDate,
    /// The last date.
    pub last_date: NaiveDate,
    /// The date the book was resolved on: the first after the warm-up.
    pub start_date: NaiveDate,
    /// Dates that only fed the first volatility estimate, with no P&L.
    pub warm_up_dates: usize,
    /// Dates dropped because some symbol had no price.
    pub dropped_dates: usize,
    /// How many dates will be replayed, and so how many rows the run will have.
    pub replay_dates: usize,
    /// Dates per second when playing.
    pub speed: f64,
    /// Whether an expired option is replaced by a new one from the same portfolio row.
    pub roll_expired: bool,
    /// The starting portfolio, as read from the sample CSV.
    pub portfolio: Vec<PositionSpec>,
    /// Returns in the `VaR` window: the market config's `vol_window_days`.
    pub var_window_days: usize,
    /// Which `VaR` method is the headline.
    pub headline_var_method: VarMethod,
    /// Price breaks treated as splits.
    pub splits: Vec<BacktestSplitNote>,
    /// Corporate actions written with relative `+Nd` dates, skipped as live-mode demo events.
    pub relative_actions_skipped: usize,
    /// The config files copied into the run's `config/` folder.
    pub config_files: Vec<String>,
    /// The git commit the code was built from.
    pub git_commit: String,
}

/// One line of `events.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum BacktestEvent {
    /// A limit changed state.
    Alert(AlertEvent),
    /// An option settled at expiry.
    Settlement(SettlementEvent),
    /// An expired option was replaced.
    Roll(BacktestRollEvent),
    /// A split or dividend fired, or failed.
    CorporateAction(CorporateActionEvent),
}

/// Where a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BacktestRunState {
    /// In memory and waiting for a command.
    Paused,
    /// In memory and stepping at its speed.
    Playing,
    /// In memory and computing the rest as fast as it can.
    RunningToEnd,
    /// Every date replayed and `summary.json` written.
    Finished,
    /// On disk only, with no `summary.json`: it was stopped, freed or cut short.
    Unfinished,
}

impl BacktestRunState {
    /// Whether the run is still in memory and unfinished.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Paused | Self::Playing | Self::RunningToEnd)
    }
}

/// `GET /api/backtests/{id}`: state and progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestStatus {
    /// The run's id.
    pub id: String,
    /// Where it is.
    pub state: BacktestRunState,
    /// Whether the run's state is held in memory. False for one read back from disk.
    pub in_memory: bool,
    /// Dates per second when playing.
    pub speed: f64,
    /// Whether expired options roll.
    pub roll_expired: bool,
    /// Where the prices came from.
    pub source: BacktestSource,
    /// The first replayed date's predecessor: the date the book was resolved on.
    pub start_date: NaiveDate,
    /// The last date of the prices.
    pub last_date: NaiveDate,
    /// The most recent date replayed, or the start date before the first step.
    pub current_date: NaiveDate,
    /// Dates replayed so far.
    pub steps_done: usize,
    /// Dates to replay in all.
    pub steps_total: usize,
    /// `steps_done / steps_total`, from 0 to 1.
    pub progress: f64,
}

/// One row of `GET /api/backtests`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestListItem {
    /// The run's id.
    pub id: String,
    /// Where it is.
    pub state: BacktestRunState,
    /// Where the prices came from.
    pub source: BacktestSource,
    /// The date the book was resolved on.
    pub start_date: NaiveDate,
    /// The last date of the prices.
    pub last_date: NaiveDate,
    /// Dates replayed so far, or in all for a finished run.
    pub days: usize,
    /// Total actual P&L so far, or in all for a finished run. `None` before the first date.
    pub total_pnl: Option<f64>,
    /// When the run was created.
    pub created: DateTime<Utc>,
}

/// `GET /api/backtests/{id}/results`: everything a run has produced so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestResults {
    /// State and progress.
    pub status: BacktestStatus,
    /// How the run was set up.
    pub settings: BacktestSettings,
    /// One row per replayed date so far.
    pub rows: Vec<DailyRow>,
    /// The summary over `rows`. Final for a finished run, otherwise over the dates so far.
    pub summary: BacktestSummary,
    /// Whether `summary` covers a run that is not finished.
    pub partial: bool,
    /// Breaches, rolls, settlements and corporate actions.
    pub events: Vec<BacktestEvent>,
}
