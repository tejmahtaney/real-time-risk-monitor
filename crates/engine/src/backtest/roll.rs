//! Keeping the book alive through a replay: replacing an option that has expired.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{PositionId, add_sessions};
use portfolio::{ExpirySpec, PositionSpec, SettlementEvent};
use serde::{Deserialize, Serialize};

/// Decides what replaces an option the moment it settles.
///
/// The engine asks once per settlement, before valuing the book. An option rolled at the close of
/// date d is therefore in the risk and `VaR` computed at that close, which is the start-of-day `VaR`
/// of d + 1: it first appears in the `VaR` used for d + 1, not in the one used for d, and it earns
/// P&L from d + 1.
pub trait RollPolicy: Send {
    /// The spec of the position to open in place of `settled`, or `None` to let it go. The engine
    /// resolves it against the market as it stands after the settling tick's prices, on `date`.
    fn replacement(&mut self, settled: &SettlementEvent, date: NaiveDate) -> Option<PositionSpec>;
}

/// A rolled option, recorded in the run's `events.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct BacktestRollEvent {
    /// The simulated time of the roll: the close of `session_date`.
    pub sim_time: DateTime<Utc>,
    /// The date the roll happened.
    pub session_date: NaiveDate,
    /// The position that expired.
    pub settled: PositionId,
    /// The position opened in its place.
    pub opened: PositionId,
    /// The new option's description, such as `SPY 2026-10-16 C 600`.
    pub description: String,
}

/// Longest position id the domain accepts.
const MAX_ID_LEN: usize = 64;

/// Rolls each expired option into a new one from the same CSV row, resolved at the roll date.
///
/// The new option keeps the row's relative strike (`ATM+5%`) and relative expiry (`+30d`), its
/// quantity and any vol override, so the book stays about the same shape however long the replay
/// runs. Its entry price is left blank, which resolves to the theoretical value at the roll date.
///
/// - A `+0d` row (expires today) would settle again the moment it was opened, so it rolls to the
///   next date of the replay instead: a one-day option, opened at each close.
/// - A row with an absolute expiry date cannot be rolled, since that date has passed. It is
///   logged and let go.
/// - The new id is `<row id>-r<n>`, counting rolls of that row.
#[derive(Debug, Clone)]
pub struct SpecRoller {
    rows: HashMap<PositionId, RollRow>,
    dates: Arc<[NaiveDate]>,
}

/// The original CSV row behind a position, and how many times it has rolled.
#[derive(Debug, Clone)]
struct RollRow {
    spec: PositionSpec,
    generation: u32,
}

impl SpecRoller {
    /// A roller for the rows of the starting portfolio, over the replay's `dates` (sorted).
    pub fn new(specs: &[PositionSpec], dates: Arc<[NaiveDate]>) -> Self {
        let rows = specs
            .iter()
            .map(|spec| {
                (
                    spec.id.clone(),
                    RollRow {
                        spec: spec.clone(),
                        generation: 0,
                    },
                )
            })
            .collect();
        Self { rows, dates }
    }
}

impl RollPolicy for SpecRoller {
    fn replacement(&mut self, settled: &SettlementEvent, date: NaiveDate) -> Option<PositionSpec> {
        let row = self.rows.get(&settled.position_id)?.clone();
        let expiry = match row.spec.expiry? {
            ExpirySpec::Date(_) => {
                tracing::warn!(id = %settled.position_id, "an option with an absolute expiry date cannot be rolled");
                return None;
            }
            ExpirySpec::Today => ExpirySpec::Date(next_date(&self.dates, date)),
            relative @ ExpirySpec::FirstFridayAfter(_) => relative,
        };
        let generation = row.generation + 1;
        let id = PositionId::new(&format!("{}-r{generation}", row.spec.id)).ok()?;
        if id.as_str().len() > MAX_ID_LEN {
            tracing::warn!(id = %settled.position_id, "the rolled position's id would be too long");
            return None;
        }
        let spec = PositionSpec {
            id: id.clone(),
            entry_price: None,
            expiry: Some(expiry),
            adjusted: None,
            applied_actions: None,
            ..row.spec.clone()
        };
        // A later roll of the new option counts on from this one and starts from the row as
        // written (so a `+0d` row keeps rolling one day at a time), under the row's own id.
        self.rows.insert(
            id,
            RollRow {
                spec: row.spec,
                generation,
            },
        );
        Some(spec)
    }
}

/// The first replay date after `date`, so a one-day option lives exactly until the next date the
/// replay visits. Past the last date it falls back to the next weekday.
pub(crate) fn next_date(dates: &[NaiveDate], date: NaiveDate) -> NaiveDate {
    dates
        .iter()
        .copied()
        .find(|d| *d > date)
        .unwrap_or_else(|| add_sessions(date, 1))
}
