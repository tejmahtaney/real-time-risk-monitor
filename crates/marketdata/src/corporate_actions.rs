//! The corporate-action calendar: `config/corporate_actions.toml`, date resolution, and
//! replaying the ledger of applied actions into a seed.
//!
//! Dates are written as an absolute `YYYY-MM-DD` or as `+Nd`, meaning N sessions after the
//! simulation start (weekdays only, holidays ignored like the rest of the simulated clock).
//! The simulation always restarts on the same date, so a `+2d` event is always still ahead
//! at startup and the demo always has something coming up.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use domain::{
    AppliedAction, CashDividend, CorporateAction, DomainError, SpotAdjustment, Symbol, add_sessions,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{DividendModel, MarketConfig};
use crate::seed::SeedData;

/// The corporate-action configuration or ledger could not be used.
#[derive(Debug, Error)]
pub enum CorporateActionError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The TOML did not parse into the expected shape.
    #[error("invalid corporate actions config: {0}")]
    Parse(#[from] toml::de::Error),
    /// A value broke a rule.
    #[error("invalid corporate actions config: {0}")]
    Invalid(String),
    /// The ledger of applied actions could not be replayed into the seed.
    #[error("cannot replay applied action {id}: {message}")]
    Ledger {
        /// The action id.
        id: String,
        /// What went wrong.
        message: String,
    },
}

/// A date as written in the config: absolute, or a number of sessions after the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EventDate {
    /// A fixed calendar date, `2026-10-02`.
    Absolute(NaiveDate),
    /// `+2d`: two sessions after the simulation start.
    Sessions(u32),
}

impl FromStr for EventDate {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = s.trim();
        if let Some(count) = text.strip_prefix('+').and_then(|t| t.strip_suffix('d')) {
            let sessions: u32 = count
                .parse()
                .map_err(|_| format!("expected +Nd with a whole number N, got {text:?}"))?;
            if sessions == 0 {
                return Err("+0d is the start session itself, which has already opened".into());
            }
            return Ok(Self::Sessions(sessions));
        }
        NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(Self::Absolute)
            .map_err(|_| format!("expected YYYY-MM-DD or +Nd, got {text:?}"))
    }
}

impl fmt::Display for EventDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absolute(date) => write!(f, "{date}"),
            Self::Sessions(n) => write!(f, "+{n}d"),
        }
    }
}

impl TryFrom<String> for EventDate {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<EventDate> for String {
    fn from(date: EventDate) -> Self {
        date.to_string()
    }
}

impl EventDate {
    /// The session date, counting relative dates from `start`.
    pub fn resolve(self, start: NaiveDate) -> NaiveDate {
        match self {
            Self::Absolute(date) => date,
            Self::Sessions(n) => add_sessions(start, n),
        }
    }
}

/// A split as written in the config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitEntry {
    /// The action id, unique in the file. It is what the ledger and each position's
    /// `applied_actions` refer to, so keep it stable.
    pub id: String,
    /// The underlying that splits.
    pub symbol: Symbol,
    /// New shares per old share: 4.0 is 4-for-1, 1.5 is 3-for-2, 0.1 is 1-for-10.
    pub ratio: f64,
    /// The session that opens post-split.
    pub effective: EventDate,
}

/// A cash dividend as written in the config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DividendEntry {
    /// The action id, unique in the file.
    pub id: String,
    /// The underlying that pays. It must be on the discrete dividend model.
    pub symbol: Symbol,
    /// Cash per share, in dollars, in the terms of the day the config is written.
    pub amount: f64,
    /// The first session that trades without the dividend.
    pub ex_date: EventDate,
}

/// The optional Alpha Vantage loader of real upcoming events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionsSourceConfig {
    /// Whether to fetch real upcoming splits and dividends. Off by default: the free tier
    /// allows 25 requests a day, and real events say nothing about a simulated timeline.
    #[serde(default)]
    pub enabled: bool,
}

/// The contents of `config/corporate_actions.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporateActionsConfig {
    /// The optional real-data loader.
    #[serde(default)]
    pub alpha_vantage: ActionsSourceConfig,
    /// The splits.
    #[serde(default, rename = "split")]
    pub splits: Vec<SplitEntry>,
    /// The cash dividends.
    #[serde(default, rename = "dividend")]
    pub dividends: Vec<DividendEntry>,
}

impl CorporateActionsConfig {
    /// Reads and validates a TOML file.
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Read`], [`CorporateActionError::Parse`] or
    /// [`CorporateActionError::Invalid`].
    pub fn load(path: &Path) -> Result<Self, CorporateActionError> {
        let text = std::fs::read_to_string(path).map_err(|source| CorporateActionError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Parses and validates TOML text.
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Parse`] or [`CorporateActionError::Invalid`].
    pub fn from_toml_str(text: &str) -> Result<Self, CorporateActionError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Checks ids are non-empty, made of letters, digits, `.`, `_` and `-`, and unique,
    /// and that each ratio and amount is usable.
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Invalid`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), CorporateActionError> {
        let mut seen = BTreeSet::new();
        let ids = self
            .splits
            .iter()
            .map(|s| &s.id)
            .chain(self.dividends.iter().map(|d| &d.id));
        for id in ids {
            let valid = !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
            if !valid {
                return Err(CorporateActionError::Invalid(format!(
                    "action id {id:?} must be 1 to 64 characters of letters, digits, '.', '_' or '-'"
                )));
            }
            if !seen.insert(id.clone()) {
                return Err(CorporateActionError::Invalid(format!(
                    "action id {id:?} is used twice"
                )));
            }
        }
        // Building each action checks its ratio or amount.
        let placeholder = NaiveDate::MIN;
        for split in &self.splits {
            CorporateAction::split(split.symbol.clone(), split.ratio, placeholder)
                .map_err(|e| invalid_entry(&split.id, &e))?;
        }
        for dividend in &self.dividends {
            CorporateAction::cash_dividend(dividend.symbol.clone(), dividend.amount, placeholder)
                .map_err(|e| invalid_entry(&dividend.id, &e))?;
        }
        Ok(())
    }
}

fn invalid_entry(id: &str, error: &DomainError) -> CorporateActionError {
    CorporateActionError::Invalid(format!("action {id}: {error}"))
}

/// An action with its id, ready to fire on its date.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAction {
    /// The action id.
    pub id: String,
    /// The action, with its date resolved.
    pub action: CorporateAction,
}

/// What [`CorporateActionCalendar::resolve_dated`] left out of a replay's calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DatedActions {
    /// Entries written as `+Nd`, which belong to live mode.
    pub relative: usize,
    /// Absolute entries before the start or after the end of the replay.
    pub outside_window: usize,
}

/// The actions still to come, in date order, with dates resolved against the start.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CorporateActionCalendar {
    actions: Vec<ScheduledAction>,
}

impl CorporateActionCalendar {
    /// Resolves the config's dates against `start`, the date of the first simulated session.
    /// An absolute date on or before `start` can never fire (actions apply at a later open)
    /// and is dropped with a warning.
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Invalid`] for a ratio or amount the domain rejects.
    pub fn resolve(
        config: &CorporateActionsConfig,
        start: NaiveDate,
    ) -> Result<Self, CorporateActionError> {
        let mut actions = Vec::new();
        for split in &config.splits {
            let action = CorporateAction::split(
                split.symbol.clone(),
                split.ratio,
                split.effective.resolve(start),
            )
            .map_err(|e| invalid_entry(&split.id, &e))?;
            actions.push(ScheduledAction {
                id: split.id.clone(),
                action,
            });
        }
        for dividend in &config.dividends {
            let action = CorporateAction::cash_dividend(
                dividend.symbol.clone(),
                dividend.amount,
                dividend.ex_date.resolve(start),
            )
            .map_err(|e| invalid_entry(&dividend.id, &e))?;
            actions.push(ScheduledAction {
                id: dividend.id.clone(),
                action,
            });
        }
        let mut calendar = Self { actions };
        calendar.retain_after(start);
        Ok(calendar)
    }

    /// Resolves only the actions written with an absolute date that falls inside a replay:
    /// after `start` (actions apply at a later open) and on or before `end`.
    ///
    /// Relative `+Nd` entries are demo events for the live simulation, which always starts on the
    /// same date; they mean nothing on a historical timeline and are skipped. The returned
    /// [`DatedActions`] counts what was left out so the caller can log it. The optional
    /// Alpha Vantage loader is not consulted.
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Invalid`] for a ratio or amount the domain rejects.
    pub fn resolve_dated(
        config: &CorporateActionsConfig,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<(Self, DatedActions), CorporateActionError> {
        let mut left_out = DatedActions::default();
        let mut actions = Vec::new();
        let mut window = |id: &str, date: EventDate| match date {
            EventDate::Sessions(_) => {
                left_out.relative += 1;
                tracing::info!(%id, "relative corporate action skipped: it belongs to live mode");
                None
            }
            EventDate::Absolute(day) if day <= start || day > end => {
                left_out.outside_window += 1;
                tracing::info!(%id, date = %day, "corporate action is outside the replay window");
                None
            }
            EventDate::Absolute(day) => Some(day),
        };
        for split in &config.splits {
            if let Some(day) = window(&split.id, split.effective) {
                let action = CorporateAction::split(split.symbol.clone(), split.ratio, day)
                    .map_err(|e| invalid_entry(&split.id, &e))?;
                actions.push(ScheduledAction {
                    id: split.id.clone(),
                    action,
                });
            }
        }
        for dividend in &config.dividends {
            if let Some(day) = window(&dividend.id, dividend.ex_date) {
                let action =
                    CorporateAction::cash_dividend(dividend.symbol.clone(), dividend.amount, day)
                        .map_err(|e| invalid_entry(&dividend.id, &e))?;
                actions.push(ScheduledAction {
                    id: dividend.id.clone(),
                    action,
                });
            }
        }
        let mut calendar = Self { actions };
        calendar.sort();
        Ok((calendar, left_out))
    }

    /// Keeps only the actions for which `keep` is true.
    pub fn retain(&mut self, mut keep: impl FnMut(&ScheduledAction) -> bool) {
        self.actions.retain(|scheduled| keep(scheduled));
    }

    fn retain_after(&mut self, start: NaiveDate) {
        self.actions.retain(|scheduled| {
            let keep = scheduled.action.date() > start;
            if !keep {
                tracing::warn!(
                    id = %scheduled.id,
                    date = %scheduled.action.date(),
                    %start,
                    "corporate action is on or before the simulation start and will not fire"
                );
            }
            keep
        });
        self.sort();
    }

    /// Date order, splits before dividends on the same date, otherwise config order.
    fn sort(&mut self) {
        self.actions.sort_by_key(|scheduled| {
            let rank = match scheduled.action {
                CorporateAction::Split { .. } => 0,
                CorporateAction::CashDividend { .. } => 1,
            };
            (scheduled.action.date(), rank)
        });
    }

    /// Adds more actions, such as ones loaded from Alpha Vantage. An id already present is
    /// skipped, so a real event cannot shadow a configured one.
    pub fn merge(&mut self, extra: impl IntoIterator<Item = ScheduledAction>) {
        for scheduled in extra {
            if self.actions.iter().all(|a| a.id != scheduled.id) {
                self.actions.push(scheduled);
            }
        }
        self.sort();
    }

    /// Checks every action names a symbol in the market config, and that each dividend is
    /// on a symbol using the discrete model (a yield-model symbol has no schedule to pay from).
    ///
    /// # Errors
    ///
    /// [`CorporateActionError::Invalid`] naming the first offender.
    pub fn validate_against(&self, market: &MarketConfig) -> Result<(), CorporateActionError> {
        for scheduled in &self.actions {
            let symbol = scheduled.action.symbol();
            if !market.symbols.contains(symbol) {
                return Err(CorporateActionError::Invalid(format!(
                    "action {} is for {symbol}, which is not in market.toml's symbols",
                    scheduled.id
                )));
            }
            if matches!(scheduled.action, CorporateAction::CashDividend { .. })
                && market.model_of(symbol) != DividendModel::Discrete
            {
                return Err(CorporateActionError::Invalid(format!(
                    "dividend {} is for {symbol}, which must be set to \"discrete\" in market.toml's dividend_model",
                    scheduled.id
                )));
            }
        }
        Ok(())
    }

    /// Drops every action whose id is in the ledger: it has already been applied.
    pub fn without_applied(&mut self, ledger: &[AppliedAction]) {
        self.actions
            .retain(|scheduled| ledger.iter().all(|applied| applied.id != scheduled.id));
    }

    /// Removes and returns the actions due on or before `session_date`, in firing order.
    pub fn take_due(&mut self, session_date: NaiveDate) -> Vec<ScheduledAction> {
        let (due, pending): (Vec<_>, Vec<_>) = self
            .actions
            .drain(..)
            .partition(|scheduled| scheduled.action.date() <= session_date);
        self.actions = pending;
        due
    }

    /// The actions still to come.
    pub fn pending(&self) -> &[ScheduledAction] {
        &self.actions
    }

    /// The pending dividend schedule for `discrete` symbols, each divided by the ratio of
    /// every ledgered split on that symbol effective on or before its ex-date, since a
    /// share of a 4-for-1 split pays a quarter of the old dividend. A symbol with nothing
    /// pending still gets an empty entry, which is what puts it on the discrete model.
    pub fn dividend_schedule(
        &self,
        discrete: &[Symbol],
        ledger: &[AppliedAction],
    ) -> BTreeMap<Symbol, Vec<CashDividend>> {
        let mut schedule: BTreeMap<Symbol, Vec<CashDividend>> =
            discrete.iter().map(|s| (s.clone(), Vec::new())).collect();
        for scheduled in &self.actions {
            let CorporateAction::CashDividend {
                symbol,
                amount,
                ex_date,
            } = &scheduled.action
            else {
                continue;
            };
            let Some(entries) = schedule.get_mut(symbol) else {
                continue;
            };
            let split_ratio: f64 = ledger
                .iter()
                .filter_map(|applied| match &applied.action {
                    CorporateAction::Split {
                        symbol: split_symbol,
                        effective,
                        ..
                    } if split_symbol == symbol && effective <= ex_date => Some(applied.amount),
                    _ => None,
                })
                .product();
            entries.push(CashDividend {
                ex_date: *ex_date,
                amount: amount / split_ratio,
            });
        }
        schedule
    }
}

/// Replays the ledger into a seed, so the market and simulator built from it start in the
/// same terms as a book restored from `current.json` or a saved version.
///
/// Each split divides the symbol's spot by its ratio and each cash dividend subtracts its
/// amount, in ledger order. This starts from the seed's calibrated spot, not from the price
/// the simulated path had reached when the action fired, which is close enough for a
/// restart and is noted as such in the docs.
///
/// # Errors
///
/// [`CorporateActionError::Ledger`] for an action on a symbol the seed does not hold, or one
/// that would leave a spot at or below zero. The seed is unchanged on error.
pub fn apply_ledger_to_seed(
    seed: &mut SeedData,
    ledger: &[AppliedAction],
) -> Result<(), CorporateActionError> {
    let mut staged = seed.clone();
    for applied in ledger {
        let ledger_error = |message: String| CorporateActionError::Ledger {
            id: applied.id.clone(),
            message,
        };
        let adjustment = match applied.action {
            CorporateAction::Split { .. } => SpotAdjustment::DivideBy(applied.amount),
            CorporateAction::CashDividend { .. } => SpotAdjustment::Subtract(applied.amount),
        };
        let symbol = applied.action.symbol();
        let underlying = staged
            .underlyings
            .get_mut(symbol)
            .ok_or_else(|| ledger_error(format!("the seed has no data for {symbol}")))?;
        underlying.spot = adjustment
            .apply(underlying.spot)
            .map_err(|e| ledger_error(e.to_string()))?;
    }
    *seed = staged;
    Ok(())
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    const TOML: &str = r#"
        [alpha_vantage]
        enabled = false

        [[split]]
        id = "aapl-4-for-1"
        symbol = "AAPL"
        ratio = 4.0
        effective = "+2d"

        [[split]]
        id = "tsla-3-for-2"
        symbol = "TSLA"
        ratio = 1.5
        effective = "2026-10-08"

        [[dividend]]
        id = "msft-q"
        symbol = "MSFT"
        amount = 0.98
        ex_date = "+3d"
    "#;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Wednesday 30 September 2026.
    fn start() -> NaiveDate {
        date(2026, 9, 30)
    }

    #[test]
    fn dates_parse_and_resolve() {
        assert_eq!("+2d".parse::<EventDate>(), Ok(EventDate::Sessions(2)));
        assert_eq!(
            "2026-10-02".parse::<EventDate>(),
            Ok(EventDate::Absolute(date(2026, 10, 2)))
        );
        for bad in ["+0d", "+d", "2d", "+2", "tomorrow", "2026-13-01", ""] {
            assert!(bad.parse::<EventDate>().is_err(), "{bad:?}");
        }
        // Two sessions after Wednesday is Friday, three is the following Monday.
        assert_eq!(EventDate::Sessions(2).resolve(start()), date(2026, 10, 2));
        assert_eq!(EventDate::Sessions(3).resolve(start()), date(2026, 10, 5));
    }

    #[test]
    fn the_calendar_resolves_sorts_and_drops_past_dates() {
        let config = CorporateActionsConfig::from_toml_str(TOML).unwrap();
        let calendar = CorporateActionCalendar::resolve(&config, start()).unwrap();
        let ids: Vec<_> = calendar.pending().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["aapl-4-for-1", "msft-q", "tsla-3-for-2"]);
        assert_eq!(calendar.pending()[0].action.date(), date(2026, 10, 2));
        // From a later start the absolute date is in the past and disappears.
        let later = CorporateActionCalendar::resolve(&config, date(2026, 10, 8)).unwrap();
        assert!(later.pending().iter().all(|a| a.id != "tsla-3-for-2"));
    }

    #[test]
    fn a_replay_calendar_takes_only_absolute_dates_inside_the_window() {
        let config = CorporateActionsConfig::from_toml_str(TOML).unwrap();
        // The relative demo events are for live mode. Only the TSLA split has an absolute date.
        let (calendar, left_out) =
            CorporateActionCalendar::resolve_dated(&config, start(), date(2026, 12, 31)).unwrap();
        let ids: Vec<_> = calendar.pending().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["tsla-3-for-2"]);
        assert_eq!(
            left_out,
            DatedActions {
                relative: 2,
                outside_window: 0
            }
        );
        // A window that ends before it, or starts on it, leaves it out.
        let (before, out) =
            CorporateActionCalendar::resolve_dated(&config, start(), date(2026, 10, 7)).unwrap();
        assert!(before.pending().is_empty());
        assert_eq!(out.outside_window, 1);
        let (on_start, _) =
            CorporateActionCalendar::resolve_dated(&config, date(2026, 10, 8), date(2026, 12, 1))
                .unwrap();
        assert!(on_start.pending().is_empty());
        // The end date itself is inside the window.
        let (on_end, _) =
            CorporateActionCalendar::resolve_dated(&config, start(), date(2026, 10, 8)).unwrap();
        assert_eq!(on_end.pending().len(), 1);
    }

    #[test]
    fn take_due_removes_actions_in_firing_order() {
        let config = CorporateActionsConfig::from_toml_str(TOML).unwrap();
        let mut calendar = CorporateActionCalendar::resolve(&config, start()).unwrap();
        assert!(calendar.take_due(date(2026, 10, 1)).is_empty());
        let due = calendar.take_due(date(2026, 10, 5));
        assert_eq!(
            due.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["aapl-4-for-1", "msft-q"]
        );
        assert_eq!(calendar.pending().len(), 1);
    }

    #[test]
    fn a_split_fires_before_a_dividend_on_the_same_date() {
        let text = r#"
            [[dividend]]
            id = "d"
            symbol = "AAPL"
            amount = 1.0
            ex_date = "+2d"
            [[split]]
            id = "s"
            symbol = "AAPL"
            ratio = 2.0
            effective = "+2d"
        "#;
        let config = CorporateActionsConfig::from_toml_str(text).unwrap();
        let mut calendar = CorporateActionCalendar::resolve(&config, start()).unwrap();
        let due = calendar.take_due(date(2026, 10, 2));
        assert_eq!(due[0].id, "s");
    }

    #[test]
    fn broken_configs_are_rejected() {
        let cases = [
            TOML.replace("aapl-4-for-1", "bad id"),
            TOML.replace("tsla-3-for-2", "aapl-4-for-1"),
            TOML.replace("ratio = 4.0", "ratio = 1.0"),
            TOML.replace("ratio = 4.0", "ratio = -4.0"),
            TOML.replace("amount = 0.98", "amount = 0.0"),
            TOML.replace("\"+2d\"", "\"+0d\""),
            format!("{TOML}\nextra = 1"),
            TOML.replace("symbol = \"AAPL\"", "symbol = \"not valid\""),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert!(
                CorporateActionsConfig::from_toml_str(text).is_err(),
                "case {i}"
            );
        }
        assert!(CorporateActionsConfig::from_toml_str("").is_ok());
    }

    fn market_config(discrete: &[&str]) -> MarketConfig {
        let symbols = ["AAPL", "TSLA", "MSFT"];
        MarketConfig {
            symbols: symbols.iter().map(|s| sym(s)).collect(),
            risk_free_rate: 0.04,
            dividend_yields: symbols.iter().map(|s| (sym(s), 0.0)).collect(),
            dividend_model: discrete
                .iter()
                .map(|s| (sym(s), DividendModel::Discrete))
                .collect(),
            vol_window_days: 60,
            vol_method: crate::config::VolMethod::Realised,
            cache_max_age_hours: 24,
            data_mode: crate::config::DataMode::Offline,
        }
    }

    #[test]
    fn a_dividend_needs_the_discrete_model_and_a_listed_symbol() {
        let config = CorporateActionsConfig::from_toml_str(TOML).unwrap();
        let calendar = CorporateActionCalendar::resolve(&config, start()).unwrap();
        assert!(calendar.validate_against(&market_config(&["MSFT"])).is_ok());
        assert!(calendar.validate_against(&market_config(&[])).is_err());
        let mut unlisted = market_config(&["MSFT"]);
        unlisted.symbols.retain(|s| s.as_str() != "TSLA");
        unlisted.dividend_yields.remove(&sym("TSLA"));
        assert!(calendar.validate_against(&unlisted).is_err());
    }

    fn applied(id: &str, action: CorporateAction, amount: f64) -> AppliedAction {
        AppliedAction {
            id: id.to_owned(),
            action,
            amount,
        }
    }

    #[test]
    fn the_ledger_removes_applied_actions_and_scales_pending_dividends() {
        let config = CorporateActionsConfig::from_toml_str(TOML).unwrap();
        let mut calendar = CorporateActionCalendar::resolve(&config, start()).unwrap();
        let split = applied(
            "aapl-4-for-1",
            CorporateAction::split(sym("AAPL"), 4.0, date(2026, 10, 2)).unwrap(),
            4.0,
        );
        // A split of MSFT before the dividend's ex-date halves the dividend.
        let msft_split = applied(
            "msft-2-for-1",
            CorporateAction::split(sym("MSFT"), 2.0, date(2026, 10, 1)).unwrap(),
            2.0,
        );
        let ledger = vec![split, msft_split];
        calendar.without_applied(&ledger);
        assert!(calendar.pending().iter().all(|a| a.id != "aapl-4-for-1"));
        let schedule = calendar.dividend_schedule(&[sym("MSFT"), sym("AAPL")], &ledger);
        assert_relative_eq!(schedule[&sym("MSFT")][0].amount, 0.49);
        // A discrete symbol with nothing pending still has an (empty) entry.
        assert!(schedule[&sym("AAPL")].is_empty());
    }

    #[test]
    fn replaying_the_ledger_adjusts_the_seed_spots_and_is_atomic() {
        use crate::loader::DataSource;
        use crate::seed::SymbolSeed;
        let seed_for = |spot| SymbolSeed {
            spot,
            vol_realised: 0.2,
            vol_ewma: 0.2,
            vol: 0.2,
            dividend_yield: 0.0,
            source: DataSource::Fallback {
                file: "x.csv".to_owned(),
            },
        };
        let mut seed = SeedData {
            as_of: start(),
            rate: 0.04,
            symbols: vec![sym("AAPL"), sym("MSFT")],
            underlyings: BTreeMap::from([
                (sym("AAPL"), seed_for(200.0)),
                (sym("MSFT"), seed_for(400.0)),
            ]),
            return_dates: Vec::new(),
            returns: Vec::new(),
            correlation: Vec::new(),
            dividend_schedule: BTreeMap::new(),
        };
        let split = applied(
            "s",
            CorporateAction::split(sym("AAPL"), 4.0, date(2026, 10, 2)).unwrap(),
            4.0,
        );
        let dividend = applied(
            "d",
            CorporateAction::cash_dividend(sym("MSFT"), 0.98, date(2026, 10, 5)).unwrap(),
            0.98,
        );
        apply_ledger_to_seed(&mut seed, &[split.clone(), dividend]).unwrap();
        assert_relative_eq!(seed.underlyings[&sym("AAPL")].spot, 50.0);
        assert_relative_eq!(seed.underlyings[&sym("MSFT")].spot, 399.02);

        let before = seed.clone();
        let unknown = applied(
            "u",
            CorporateAction::split(sym("NVDA"), 2.0, date(2026, 10, 2)).unwrap(),
            2.0,
        );
        assert!(apply_ledger_to_seed(&mut seed, &[split, unknown]).is_err());
        assert_eq!(seed, before, "a failed replay must not half-apply");
    }
}
