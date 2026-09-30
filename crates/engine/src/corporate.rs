//! Corporate actions in the engine: applying splits and cash dividends at a session open.
//!
//! Actions fire in [`Engine::tick`] on an [`Overnight`](domain::StepKind::Overnight) step,
//! right after the feed's tick is applied and before the session's opening spots are
//! recorded, settlement, valuation and attribution. So the open spots are post-adjustment
//! and the tick's valuation already sees the adjusted book.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use domain::{
    AppliedAction, ClockStep, CorporateAction, MarketState, PositionId, SpotAdjustment, Symbol,
};
use marketdata::{CorporateActionCalendar, PriceFeed, ScheduledAction};
use portfolio::{AppliedSplit, ledger_splits};
use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::recorder::{LEDGER_FILE, RecorderRecord, SessionEvent};
use storage::{StorageError, StorageRoot};

/// How many corporate-action events a snapshot carries.
pub(crate) const RECENT_EVENTS: usize = 50;

/// What happened when an action fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum CorporateActionOutcome {
    /// The action was applied.
    Applied,
    /// The action could not be applied and was dropped. Nothing was changed.
    Failed,
}

/// An informational event: a corporate action fired. It is not a limit, so it has its own
/// type rather than a `LimitKind`. Recorded in `events.jsonl` and carried on every snapshot
/// (the last few) so the alerts pane can show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct CorporateActionEvent {
    /// The action's id.
    pub id: String,
    /// Real-world time it fired.
    pub wall_time: DateTime<Utc>,
    /// Simulated time it fired.
    pub sim_time: DateTime<Utc>,
    /// The action.
    pub action: CorporateAction,
    /// Applied, or failed.
    pub outcome: CorporateActionOutcome,
    /// A sentence for the alerts pane, such as `AAPL 4-for-1 split: 3 positions adjusted`.
    pub message: String,
    /// How many positions a split adjusted. Zero for a dividend.
    pub positions_adjusted: u32,
    /// Dividend cash paid to stock positions, negative when net short. Zero for a split.
    pub cash_paid: f64,
}

/// The corporate-action inputs an engine starts with.
#[derive(Debug, Clone, Default)]
pub struct CorporateActionState {
    /// The actions still to come.
    pub calendar: CorporateActionCalendar,
    /// The actions already applied, in the order they were applied. On a `last` start this is
    /// what the persisted ledger holds. It is how a loaded book is brought up to date: any
    /// ledgered split a position does not yet list in `applied_actions` is applied to it.
    pub ledger: Vec<AppliedAction>,
}

/// Reads the ledger of applied corporate actions from `portfolios/applied-actions.jsonl`,
/// oldest first. A missing file is an empty ledger.
///
/// # Errors
///
/// [`StorageError`] when the file exists but cannot be read or holds a bad line.
pub fn read_ledger(storage: &StorageRoot) -> Result<Vec<AppliedAction>, StorageError> {
    storage::read_jsonl(&storage.join(&["portfolios", LEDGER_FILE])?)
}

/// Sets the ledger aside as `portfolios/applied-actions.<UTC stamp>.jsonl` and returns the new
/// path, or `None` when there was no ledger. A `sample` start does this: the sample book is
/// unadjusted, so the old ledger no longer describes it. It is renamed, never deleted.
///
/// # Errors
///
/// [`StorageError`] when the rename fails.
pub fn archive_ledger(
    storage: &StorageRoot,
    now: DateTime<Utc>,
) -> Result<Option<PathBuf>, StorageError> {
    let path = storage.join(&["portfolios", LEDGER_FILE])?;
    if !path.exists() {
        return Ok(None);
    }
    let name = format!("applied-actions.{}.jsonl", storage::timestamp_name(now));
    let target = storage.join(&["portfolios", &name])?;
    std::fs::rename(&path, &target).map_err(|source| StorageError::Io {
        action: "rename",
        path: path.clone(),
        source,
    })?;
    Ok(Some(target))
}

/// The splits in `ledger` on `symbol` that `applied` does not list, in ledger order.
pub(crate) fn missing_splits<'a>(
    splits: &'a [AppliedSplit],
    symbol: &'a Symbol,
    applied: &'a [String],
) -> impl Iterator<Item = &'a AppliedSplit> {
    splits
        .iter()
        .filter(move |s| &s.symbol == symbol && !applied.contains(&s.id))
}

impl<F: PriceFeed> Engine<F> {
    /// The ledgered splits, for loading books.
    pub(crate) fn applied_splits(&self) -> Vec<AppliedSplit> {
        ledger_splits(&self.ledger)
    }

    /// Fires every action due on `step.session_date` or earlier.
    ///
    /// `prior_market` is the market exactly as the **previous tick left it**: its spots, vols and
    /// dividend schedule from before this tick's feed step and before any action of this tick. It is
    /// cloned by [`Engine::tick`] at the top, before `feed.step`. A split then brings that
    /// clone into post-split units (spot ÷ ratio, pending dividends ÷ ratio, nothing else) so
    /// the attributor can be rebased to "the previous tick, revalued under the adjusted terms":
    /// the adjusted book valued at `step.from` on this market. That row diffs cleanly against
    /// this tick's valuation, so only what really happened (the overnight decay) is
    /// attributed and the split itself shows no P&L.
    pub(crate) fn apply_due_actions(&mut self, step: &ClockStep, mut prior_market: MarketState) {
        let due = self.calendar.take_due(step.session_date);
        if due.is_empty() {
            return;
        }
        let mut split_applied = false;
        for scheduled in due {
            let result = match scheduled.action {
                CorporateAction::Split { .. } => {
                    self.apply_split(&scheduled, step, &mut prior_market)
                }
                CorporateAction::CashDividend { .. } => self.apply_dividend(&scheduled),
            };
            match result {
                Ok(done) => {
                    split_applied |= matches!(scheduled.action, CorporateAction::Split { .. });
                    let applied = AppliedAction {
                        id: scheduled.id.clone(),
                        action: scheduled.action.clone(),
                        amount: done.amount,
                    };
                    self.ledger.push(applied.clone());
                    self.pending_records
                        .push(RecorderRecord::AppliedAction(applied));
                    let message = format!("{}: {}", scheduled.action.describe(), done.summary);
                    self.push_corporate_event(
                        &scheduled,
                        step,
                        CorporateActionOutcome::Applied,
                        message,
                        done.positions_adjusted,
                        done.cash_paid,
                    );
                }
                Err(reason) => {
                    tracing::error!(id = %scheduled.id, %reason, "corporate action failed and was dropped; nothing was changed");
                    let message = format!(
                        "{} could not be applied: {reason}",
                        scheduled.action.describe()
                    );
                    self.push_corporate_event(
                        &scheduled,
                        step,
                        CorporateActionOutcome::Failed,
                        message,
                        0,
                        0.0,
                    );
                }
            }
        }
        if split_applied {
            self.pending_records
                .push(RecorderRecord::CurrentPortfolio(self.active_positions()));
        }
    }

    fn push_corporate_event(
        &mut self,
        scheduled: &ScheduledAction,
        step: &ClockStep,
        outcome: CorporateActionOutcome,
        message: String,
        positions_adjusted: u32,
        cash_paid: f64,
    ) {
        let event = CorporateActionEvent {
            id: scheduled.id.clone(),
            wall_time: Utc::now(),
            sim_time: step.to,
            action: scheduled.action.clone(),
            outcome,
            message,
            positions_adjusted,
            cash_paid,
        };
        self.pending_records
            .push(RecorderRecord::Event(SessionEvent::CorporateAction(
                event.clone(),
            )));
        if self.corporate_events.len() >= RECENT_EVENTS {
            self.corporate_events.pop_front();
        }
        self.corporate_events.push_back(event);
    }

    /// Applies a split to the feed, the market, the dividend schedule and the book, and
    /// rebases the attributor. All or nothing: a failure part-way undoes the feed and market.
    fn apply_split(
        &mut self,
        scheduled: &ScheduledAction,
        step: &ClockStep,
        prior_market: &mut MarketState,
    ) -> Result<Done, String> {
        let CorporateAction::Split { symbol, ratio, .. } = &scheduled.action else {
            return Err("not a split".to_owned());
        };
        let (symbol, ratio) = (symbol.clone(), *ratio);
        let forward = SpotAdjustment::DivideBy(ratio);
        let back = SpotAdjustment::DivideBy(1.0 / ratio);

        self.feed
            .adjust_spot(&symbol, forward)
            .map_err(|e| e.to_string())?;
        if let Err(error) = self.market.adjust_spot(&symbol, forward) {
            // The feed and the market hold the same spot, so this cannot differ from the feed's
            // own check. Undo the feed all the same.
            let _ = self.feed.adjust_spot(&symbol, back);
            return Err(error.to_string());
        }
        let adjusted = match self.book.apply_split(&symbol, ratio, &scheduled.id) {
            Ok(ids) => ids,
            Err(error) => {
                let _ = self.feed.adjust_spot(&symbol, back);
                let _ = self.market.adjust_spot(&symbol, back);
                return Err(error.to_string());
            }
        };
        self.market.divide_dividends(&symbol, ratio);

        // Put the previous tick's market into post-split units too, then value the adjusted
        // book on it at the previous tick's time.
        if let Err(error) = prior_market.adjust_spot(&symbol, forward) {
            tracing::warn!(%error, "cannot bring the previous market into post-split units; the attributor is not rebased");
        } else {
            prior_market.divide_dividends(&symbol, ratio);
            let rows: Vec<_> = self
                .book
                .value_all(prior_market, step.from)
                .into_iter()
                .filter(|row| adjusted.contains(&row.id))
                .collect();
            self.attributor.rebase(&rows);
        }
        let count = u32::try_from(adjusted.len()).unwrap_or(u32::MAX);
        Ok(Done {
            amount: ratio,
            positions_adjusted: count,
            cash_paid: 0.0,
            summary: format!("{count} positions adjusted"),
        })
    }

    /// Drops the spot by the dividend, pays stock holders and tells the attributor.
    fn apply_dividend(&mut self, scheduled: &ScheduledAction) -> Result<Done, String> {
        let CorporateAction::CashDividend {
            symbol, ex_date, ..
        } = &scheduled.action
        else {
            return Err("not a dividend".to_owned());
        };
        // The amount comes from the market's schedule, not the calendar, because a split
        // since the config was written has divided it.
        let amount = self
            .market
            .dividends(symbol)
            .and_then(|schedule| schedule.iter().find(|d| d.ex_date == *ex_date))
            .map(|d| d.amount)
            .ok_or_else(|| format!("{symbol} has no scheduled dividend on {ex_date}"))?;
        let drop = SpotAdjustment::Subtract(amount);
        let spot = self
            .market
            .underlying(symbol)
            .map(domain::UnderlyingState::spot)
            .ok_or_else(|| format!("no market data for {symbol}"))?;
        drop.apply(spot).map_err(|e| e.to_string())?;

        self.feed
            .adjust_spot(symbol, drop)
            .map_err(|e| e.to_string())?;
        if let Err(error) = self.market.adjust_spot(symbol, drop) {
            let _ = self
                .feed
                .adjust_spot(symbol, SpotAdjustment::Subtract(-amount));
            return Err(error.to_string());
        }
        let _ = self.market.take_dividend_on(symbol, *ex_date);

        let payments: Vec<(PositionId, f64)> = self.book.pay_dividend(symbol, amount);
        // `+ 0.0` turns the -0.0 that summing nothing gives into a plain zero.
        let cash = payments.iter().map(|(_, cash)| cash).sum::<f64>() + 0.0;
        let summary = if payments.is_empty() {
            "no stock positions to pay".to_owned()
        } else {
            format!("{cash:.2} cash paid to stock holders")
        };
        self.attributor.note_dividend(symbol, amount, payments);
        Ok(Done {
            amount,
            positions_adjusted: 0,
            cash_paid: cash,
            summary,
        })
    }
}

/// What an applied action reports back.
struct Done {
    amount: f64,
    positions_adjusted: u32,
    cash_paid: f64,
    summary: String,
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};

    use super::*;

    fn applied(id: &str) -> AppliedAction {
        AppliedAction {
            id: id.to_owned(),
            action: CorporateAction::split(
                Symbol::new("AAPL").unwrap(),
                4.0,
                NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            )
            .unwrap(),
            amount: 4.0,
        }
    }

    #[test]
    fn a_missing_ledger_is_empty_and_archiving_it_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageRoot::at(dir.path());
        assert!(read_ledger(&storage).unwrap().is_empty());
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        assert_eq!(archive_ledger(&storage, now).unwrap(), None);
    }

    #[test]
    fn archiving_renames_the_ledger_and_keeps_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageRoot::at(dir.path());
        let folder = storage.subdir(&["portfolios"]).unwrap();
        storage::append_jsonl(&folder.join(LEDGER_FILE), &[applied("a"), applied("b")]).unwrap();
        assert_eq!(read_ledger(&storage).unwrap().len(), 2);

        let now = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        let moved = archive_ledger(&storage, now).unwrap().unwrap();
        assert!(moved.ends_with("applied-actions.20261001T090000Z.jsonl"));
        assert!(
            read_ledger(&storage).unwrap().is_empty(),
            "the live ledger is gone"
        );
        let kept: Vec<AppliedAction> = storage::read_jsonl(&moved).unwrap();
        assert_eq!(kept.len(), 2, "nothing is deleted");
    }
}
