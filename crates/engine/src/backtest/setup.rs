//! Turning an upload or the archive into a ready-to-run engine.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use domain::{CorporateAction, ReplayClock, Symbol};
use marketdata::{
    Archive, CorporateActionCalendar, CorporateActionsConfig, DividendModel, MarketConfig,
    PriceBreak, PriceTable, ROLLING_VOL_WINDOW, RawPrices, ReplayDataError, ReplayFeed,
    SystemClock, WARM_UP_DATES,
};
use portfolio::{ExpirySpec, PositionSpec, load_csv};
use risk::{LimitsConfig, VarMethod};
use storage::StorageRoot;

use super::error::BacktestError;
use super::roll::{SpecRoller, next_date};
use super::types::{BacktestSettings, BacktestSource, BacktestSplitHandling, BacktestSplitNote};
use crate::corporate::CorporateActionState;
use crate::engine::Engine;
use crate::recorder::git_commit;

/// Dates per second a run plays at until told otherwise.
pub const DEFAULT_SPEED: f64 = 5.0;
/// The slowest speed a client may set, in dates per second.
pub const MIN_SPEED: f64 = 1.0;
/// The fastest speed a client may set, in dates per second.
pub const MAX_SPEED: f64 = 50.0;

/// Everything a run needs from the app's configuration.
#[derive(Debug, Clone)]
pub struct BacktestConfig {
    /// The storage root. Runs live under `backtests/`; the archive is `market-data/`.
    pub storage: StorageRoot,
    /// `config/market.toml`: rate, dividend yields and models, the `VaR` window length.
    pub market: MarketConfig,
    /// `config/limits.toml`.
    pub limits: LimitsConfig,
    /// The headline `VaR` method from `config/risk.toml`.
    pub headline_var_method: VarMethod,
    /// `config/corporate_actions.toml`. Only entries with absolute dates inside the replay apply.
    pub corporate_actions: CorporateActionsConfig,
    /// The sample portfolio CSV every replay starts from.
    pub portfolio_path: PathBuf,
    /// Config files copied into each run's `config/` folder.
    pub config_paths: Vec<PathBuf>,
    /// The most runs that may be in memory and unfinished at once.
    pub max_running: usize,
    /// How long a run may sit with nobody using it before it is freed from memory.
    pub idle_after: Duration,
}

impl BacktestConfig {
    /// The default limits: two running runs, freed after ten idle minutes.
    pub const DEFAULT_MAX_RUNNING: usize = 2;
    /// See [`Self::idle_after`].
    pub const DEFAULT_IDLE_AFTER: Duration = Duration::from_secs(10 * 60);
}

/// Where a run's prices come from.
#[derive(Debug, Clone)]
pub enum RunInput {
    /// An uploaded CSV, in long or wide form.
    Upload {
        /// The file name the client sent, if any.
        filename: Option<String>,
        /// The file's bytes.
        bytes: Vec<u8>,
    },
    /// Every download in the local archive, merged per symbol.
    Archive,
}

/// Choices a client makes when creating a run.
#[derive(Debug, Clone, Copy)]
pub struct CreateOptions {
    /// Replace an expired option with a new one from the same portfolio row.
    pub roll_expired: bool,
    /// Dates per second when playing.
    pub speed: f64,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            roll_expired: true,
            speed: DEFAULT_SPEED,
        }
    }
}

/// A run ready to be started: the engine at its start date, and what to save.
pub(crate) struct Prepared {
    pub engine: Engine<ReplayFeed, ReplayClock>,
    pub table: Arc<PriceTable>,
    pub settings: BacktestSettings,
    pub input: Vec<u8>,
}

/// Reads the prices, builds the table and the book, and constructs the engine at the start date.
///
/// - The portfolio is the sample CSV, resolved on the first date after the 20 warm-up dates, so
///   relative strikes and expiries work for any start date.
/// - Prices are aligned and back-adjusted for real splits (logged). A configured split that lands
///   on a detected break is left to the engine; one that does not is dropped with a warning.
/// - Corporate actions come from the config with absolute dates after the start and up to the end
///   of the data. Dividends need a symbol on the discrete model.
///
/// # Errors
///
/// [`BacktestError`] for unusable prices, a portfolio symbol with no prices, an unreadable
/// portfolio or config, or an engine that cannot be built.
pub(crate) fn prepare(
    config: &BacktestConfig,
    input: RunInput,
    options: CreateOptions,
) -> Result<Prepared, BacktestError> {
    let specs = load_csv(&config.portfolio_path)?;
    let symbols: Vec<Symbol> = specs
        .iter()
        .map(|spec| spec.underlying.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let (raw, bytes, source) = read_prices(config, input, &symbols)?;

    let mut table = PriceTable::new(&raw, &symbols)?;
    let (Some(start_date), Some(first_date), Some(last_date)) = (
        table.start_date(),
        table.dates().first().copied(),
        table.end_date(),
    ) else {
        return Err(BacktestError::Internal(
            "the price table is empty".to_owned(),
        ));
    };

    let (calendar, splits, relative_skipped) =
        window_actions(config, &mut table, start_date, last_date)?;
    let seed_symbols = table.symbols().to_vec();
    let mut seed = table.seed_at(table.start_index(), &config.market)?;
    let discrete: Vec<Symbol> = config
        .market
        .discrete_symbols()
        .into_iter()
        .filter(|s| seed_symbols.contains(s))
        .collect();
    seed.dividend_schedule = calendar.dividend_schedule(&discrete, &[]);

    // A `+0d` row would already be expired on the start date. With rolling on, start it as the
    // one-day option it rolls into, so the first day is not spent settling it.
    let start_specs: Vec<PositionSpec> = specs
        .iter()
        .map(|spec| match (options.roll_expired, spec.expiry) {
            (true, Some(ExpirySpec::Today)) => PositionSpec {
                expiry: Some(ExpirySpec::Date(next_date(table.dates(), start_date))),
                ..spec.clone()
            },
            _ => spec.clone(),
        })
        .collect();

    let table = Arc::new(table);
    let dates: Arc<[NaiveDate]> = table.dates().to_vec().into();
    let clock = ReplayClock::new(Arc::clone(&dates), table.start_index(), options.speed)
        .map_err(|e| BacktestError::Invalid(e.to_string()))?;
    let feed = ReplayFeed::new(Arc::clone(&table));
    let mut engine = Engine::new_with_actions(
        clock,
        &seed,
        feed,
        &start_specs,
        config.limits.clone(),
        Box::new(SystemClock),
        0,
        0,
        config.headline_var_method,
        CorporateActionState {
            calendar,
            ledger: Vec::new(),
        },
    )?;
    engine.compute_var_every_tick();
    if options.roll_expired {
        engine = engine.with_roll_policy(Box::new(SpecRoller::new(&specs, Arc::clone(&dates))));
    }

    let config_files = config
        .config_paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .collect();
    let settings = BacktestSettings {
        run_id: engine.session_id().to_owned(),
        created: Utc::now(),
        source,
        symbols,
        ignored_symbols: table.ignored_symbols().to_vec(),
        first_date,
        last_date,
        start_date,
        warm_up_dates: WARM_UP_DATES,
        dropped_dates: table.dropped_dates(),
        replay_dates: table.len().saturating_sub(table.start_index() + 1),
        speed: options.speed,
        roll_expired: options.roll_expired,
        portfolio: specs,
        var_window_days: config.market.vol_window_days.max(ROLLING_VOL_WINDOW),
        headline_var_method: config.headline_var_method,
        splits,
        relative_actions_skipped: relative_skipped,
        config_files,
        git_commit: git_commit(),
    };
    Ok(Prepared {
        engine,
        table,
        settings,
        input: bytes,
    })
}

/// The raw prices, the exact bytes to save as `input.csv`, and where they came from.
fn read_prices(
    config: &BacktestConfig,
    input: RunInput,
    symbols: &[Symbol],
) -> Result<(RawPrices, Vec<u8>, BacktestSource), BacktestError> {
    Ok(match input {
        RunInput::Upload { filename, bytes } => (
            RawPrices::parse_csv(&bytes)?,
            bytes,
            BacktestSource::Upload { filename },
        ),
        RunInput::Archive => {
            let archive = Archive::new(config.storage.clone());
            let mut series = Vec::with_capacity(symbols.len());
            for symbol in symbols {
                let merged = archive.merged_history(symbol)?;
                if merged.bars.is_empty() {
                    return Err(ReplayDataError::NoArchiveHistory(symbol.clone()).into());
                }
                tracing::info!(%symbol, files = merged.files_read, days = merged.bars.len(), "merged archived history");
                series.push((symbol.clone(), merged.bars));
            }
            let raw = RawPrices::from_bars(series);
            let bytes = raw.to_long_csv().into_bytes();
            (raw, bytes, BacktestSource::Archive)
        }
    })
}

/// The corporate-action calendar for the replay window, and the notes on every price break.
///
/// Splits are matched to the table's detected price breaks and the table is told which to leave
/// for the engine. Everything the calendar cannot honestly apply to replayed prices is dropped
/// with a warning.
fn window_actions(
    config: &BacktestConfig,
    table: &mut PriceTable,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(CorporateActionCalendar, Vec<BacktestSplitNote>, usize), BacktestError> {
    let (mut calendar, left_out) =
        CorporateActionCalendar::resolve_dated(&config.corporate_actions, start, end)?;
    let detected = table.detected_splits().to_vec();
    let mut kept: Vec<PriceBreak> = Vec::new();
    let symbols = table.symbols().to_vec();
    calendar.retain(|scheduled| {
        let symbol = scheduled.action.symbol();
        if !symbols.contains(symbol) {
            tracing::warn!(id = %scheduled.id, %symbol, "corporate action is for a symbol that is not replayed; skipped");
            return false;
        }
        match &scheduled.action {
            CorporateAction::Split {
                symbol,
                ratio,
                effective,
            } => {
                let matched = detected
                    .iter()
                    .any(|d| &d.symbol == symbol && d.date == *effective);
                if matched {
                    kept.push(PriceBreak {
                        symbol: symbol.clone(),
                        date: *effective,
                        ratio: *ratio,
                    });
                } else {
                    tracing::warn!(id = %scheduled.id, %symbol, date = %effective, "no price break on the configured split's date; replayed prices are real, so it is skipped");
                }
                matched
            }
            CorporateAction::CashDividend { symbol, .. } => {
                let discrete = config.market.model_of(symbol) == DividendModel::Discrete;
                if !discrete {
                    tracing::warn!(id = %scheduled.id, %symbol, "dividend is for a symbol not on the discrete model; skipped");
                }
                discrete
            }
        }
    });
    table.keep_splits(&kept)?;
    let notes = detected
        .iter()
        .map(|d| BacktestSplitNote {
            symbol: d.symbol.clone(),
            date: d.date,
            ratio: d.ratio,
            handling: if kept
                .iter()
                .any(|k| k.symbol == d.symbol && k.date == d.date)
            {
                BacktestSplitHandling::AppliedByEngine
            } else {
                BacktestSplitHandling::BackAdjusted
            },
        })
        .collect();
    Ok((calendar, notes, left_out.relative))
}
