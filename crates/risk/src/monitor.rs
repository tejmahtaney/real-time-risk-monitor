//! Risk limit monitoring: utilisation, warning and breach states with hysteresis, and
//! alert events raised only when a limit's state changes.

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Utc};
use domain::{Clock, Symbol};
use serde::{Deserialize, Serialize};

use crate::limits::LimitsConfig;
use crate::summary::RiskSummary;

/// The longest [`LimitMonitor::history`] is allowed to grow. Older events are dropped.
const HISTORY_CAPACITY: usize = 200;

/// Which limit a [`LimitStatus`] or [`AlertEvent`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum LimitKind {
    /// Portfolio |dollar delta|.
    PortfolioDollarDelta,
    /// Portfolio |dollar gamma 1 %|.
    PortfolioDollarGamma1Pct,
    /// Portfolio |vega|.
    PortfolioVega,
    /// The floor on the portfolio's negative theta.
    PortfolioThetaFloor,
    /// Portfolio gross exposure.
    GrossExposure,
    /// The largest single underlying's share of gross exposure.
    Concentration,
    /// One underlying's |dollar delta|.
    UnderlyingDollarDelta,
    /// The headline method's one-day `VaR99` (Part 18). Evaluated only when
    /// [`LimitsConfig::var_99`] is configured.
    PortfolioVar99,
}

/// The whole portfolio, or one underlying.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", content = "symbol", rename_all = "snake_case")]
pub enum Scope {
    /// The whole book.
    Portfolio,
    /// One underlying.
    Underlying(Symbol),
}

/// Where a limit's utilisation stands.
///
/// Utilisation at or above 1 is `Breach`. At or above [`LimitsConfig::warning_threshold`]
/// it is `Warning`. Below that it is `Ok`. Hysteresis stops flapping around either line:
/// once a limit reaches `Breach`, it stays there until utilisation drops below
/// [`LimitsConfig::clear_threshold`]; once it reaches `Warning` (whether from `Ok` or by
/// clearing down from `Breach`), it stays at `Warning` or above until utilisation drops
/// below [`LimitsConfig::warning_clear_threshold`]. Both rules can hold a limit at a
/// state that its raw utilisation, read alone, would no longer justify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum LimitState {
    /// Utilisation is below the warning threshold and has never reached it, or a limit
    /// already at `Warning` or `Breach` has cleared below the warning clear threshold.
    Ok,
    /// Utilisation is at or above the warning threshold and below 1, or a limit already
    /// at `Warning` or `Breach` has not yet cleared below the warning clear threshold.
    Warning,
    /// Utilisation is at or above 1, or a breach has not yet cleared below the clear
    /// threshold.
    Breach,
}

/// The next state given the current one and the fresh utilisation reading.
///
/// A limit already at `Warning` or `Breach` is "elevated": it only falls back to `Ok`
/// once utilisation drops below `warning_clear_threshold`, not merely below
/// `warning_threshold`. A limit currently `Ok` needs to reach the higher
/// `warning_threshold` to enter `Warning` in the first place.
fn next_state(
    current: LimitState,
    utilisation: f64,
    warning_clear_threshold: f64,
    warning_threshold: f64,
    clear_threshold: f64,
) -> LimitState {
    if current == LimitState::Breach && utilisation >= clear_threshold {
        return LimitState::Breach;
    }
    if utilisation >= 1.0 {
        return LimitState::Breach;
    }
    let entry_threshold = if current == LimitState::Ok {
        warning_threshold
    } else {
        warning_clear_threshold
    };
    if utilisation >= entry_threshold {
        LimitState::Warning
    } else {
        LimitState::Ok
    }
}

/// One limit's status for one scope, at one evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct LimitStatus {
    /// Which limit this is.
    pub name: LimitKind,
    /// Portfolio or a symbol.
    pub scope: Scope,
    /// The current value being measured against the limit. Some limits transform the raw
    /// risk figure first: see [`PortfolioLimits::theta_floor`](crate::PortfolioLimits::theta_floor).
    pub value: f64,
    /// The configured limit.
    pub limit: f64,
    /// `|value| / limit`.
    pub utilisation: f64,
    /// `Ok`, `Warning` or `Breach`, with hysteresis already applied.
    pub state: LimitState,
}

/// One alert: a limit changing state for one scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct AlertEvent {
    /// Monotonically increasing within one [`LimitMonitor`], starting at 0.
    pub id: u64,
    /// The real-world time the state change was observed.
    pub wall_time: DateTime<Utc>,
    /// The simulated time the state change belongs to.
    pub sim_time: DateTime<Utc>,
    /// Which limit changed state.
    pub name: LimitKind,
    /// Portfolio or a symbol.
    pub scope: Scope,
    /// The state before this evaluation.
    pub from: LimitState,
    /// The state after this evaluation.
    pub to: LimitState,
    /// The value that produced the new state.
    pub value: f64,
    /// The configured limit.
    pub limit: f64,
}

/// Evaluates a [`RiskSummary`] against a [`LimitsConfig`] on every tick, tracking each
/// limit's state so it can raise an [`AlertEvent`] only when that state changes.
pub struct LimitMonitor {
    config: LimitsConfig,
    clock: Box<dyn Clock>,
    states: BTreeMap<(LimitKind, Scope), LimitState>,
    active: BTreeMap<(LimitKind, Scope), LimitStatus>,
    history: VecDeque<AlertEvent>,
    next_id: u64,
}

impl LimitMonitor {
    /// A monitor with every limit starting in `Ok` and no history.
    ///
    /// `clock` supplies the wall-clock time stamped on each [`AlertEvent`]; production
    /// code passes a real clock, tests a fixed one.
    pub fn new(config: LimitsConfig, clock: Box<dyn Clock>) -> Self {
        Self {
            config,
            clock,
            states: BTreeMap::new(),
            active: BTreeMap::new(),
            history: VecDeque::new(),
            next_id: 0,
        }
    }

    /// The configuration this monitor evaluates against.
    pub fn config(&self) -> &LimitsConfig {
        &self.config
    }

    /// Every limit and scope currently in `Warning` or `Breach`.
    pub fn active(&self) -> impl Iterator<Item = &LimitStatus> {
        self.active.values()
    }

    /// Up to the last 200 alert events, oldest first.
    pub fn history(&self) -> impl Iterator<Item = &AlertEvent> {
        self.history.iter()
    }

    /// Evaluates every limit against `summary`, returning the events for limits whose
    /// state changed on this call. `headline_var_99` is the headline `VaR` method's
    /// current `VaR99` (Part 18), evaluated against [`LimitsConfig::var_99`] when
    /// configured. `sim_time` is stamped on each event; the wall time comes from this
    /// monitor's clock.
    pub fn evaluate(
        &mut self,
        summary: &RiskSummary,
        headline_var_99: f64,
        sim_time: DateTime<Utc>,
    ) -> Vec<AlertEvent> {
        let statuses = self.build_statuses(summary, headline_var_99);

        let wall_time = self.clock.now();
        let mut events = Vec::new();
        for status in statuses {
            let key = (status.name, status.scope.clone());
            let previous = self.states.get(&key).copied().unwrap_or(LimitState::Ok);
            if status.state != previous {
                let event = AlertEvent {
                    id: self.next_id,
                    wall_time,
                    sim_time,
                    name: status.name,
                    scope: status.scope.clone(),
                    from: previous,
                    to: status.state,
                    value: status.value,
                    limit: status.limit,
                };
                self.next_id += 1;
                self.push_history(event.clone());
                events.push(event);
            }
            self.states.insert(key.clone(), status.state);
            if status.state == LimitState::Ok {
                self.active.remove(&key);
            } else {
                self.active.insert(key, status);
            }
        }
        events
    }

    /// Every configured limit's current status against `summary`, including ones
    /// currently `Ok` — unlike [`Self::active`], which only ever holds non-`Ok`
    /// limits. Read-only: does not affect hysteresis state, `active()` or `history()`
    /// the way [`Self::evaluate`] does. `headline_var_99` is as in [`Self::evaluate`].
    pub fn statuses(&self, summary: &RiskSummary, headline_var_99: f64) -> Vec<LimitStatus> {
        self.build_statuses(summary, headline_var_99)
    }

    /// Builds one [`LimitStatus`] per limit and scope, in a fixed order (the six
    /// portfolio limits, the headline `VaR99` limit when configured, then each
    /// underlying's dollar-delta limit), regardless of state. Shared by
    /// [`Self::evaluate`] (which also tracks hysteresis, active alerts and history
    /// from the result) and [`Self::statuses`] (a read-only view for callers that just
    /// want every current utilisation, such as charting).
    fn build_statuses(&self, summary: &RiskSummary, headline_var_99: f64) -> Vec<LimitStatus> {
        let totals = &summary.totals;
        let mut statuses = vec![
            self.status_for(
                LimitKind::PortfolioDollarDelta,
                Scope::Portfolio,
                totals.dollar_delta,
                self.config.portfolio.dollar_delta,
            ),
            self.status_for(
                LimitKind::PortfolioDollarGamma1Pct,
                Scope::Portfolio,
                totals.dollar_gamma_1pct,
                self.config.portfolio.dollar_gamma_1pct,
            ),
            self.status_for(
                LimitKind::PortfolioVega,
                Scope::Portfolio,
                totals.vega,
                self.config.portfolio.vega,
            ),
            self.status_for(
                LimitKind::PortfolioThetaFloor,
                Scope::Portfolio,
                (-totals.theta).max(0.0),
                self.config.portfolio.theta_floor,
            ),
            self.status_for(
                LimitKind::GrossExposure,
                Scope::Portfolio,
                totals.gross_exposure,
                self.config.portfolio.gross_exposure,
            ),
            self.status_for(
                LimitKind::Concentration,
                Scope::Portfolio,
                summary.concentration.first().map_or(0.0, |c| c.fraction),
                self.config.portfolio.max_concentration,
            ),
        ];
        if let Some(limit) = self.config.var_99 {
            statuses.push(self.status_for(
                LimitKind::PortfolioVar99,
                Scope::Portfolio,
                headline_var_99,
                limit,
            ));
        }
        for (symbol, risk) in &summary.by_underlying {
            let limit = self.config.underlying_dollar_delta.limit_for(symbol);
            statuses.push(self.status_for(
                LimitKind::UnderlyingDollarDelta,
                Scope::Underlying(symbol.clone()),
                risk.dollar_delta,
                limit,
            ));
        }
        statuses
    }

    /// Builds the [`LimitStatus`] for one limit and scope, looking up its current state
    /// for the hysteresis rule.
    fn status_for(&self, name: LimitKind, scope: Scope, value: f64, limit: f64) -> LimitStatus {
        let key = (name, scope.clone());
        let previous = self.states.get(&key).copied().unwrap_or(LimitState::Ok);
        let utilisation = value.abs() / limit;
        let state = next_state(
            previous,
            utilisation,
            self.config.warning_clear_threshold,
            self.config.warning_threshold,
            self.config.clear_threshold,
        );
        LimitStatus {
            name,
            scope,
            value,
            limit,
            utilisation,
            state,
        }
    }

    fn push_history(&mut self, event: AlertEvent) {
        if self.history.len() >= HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use domain::FixedClock;

    use super::*;
    use crate::limits::{PortfolioLimits, UnderlyingDeltaLimit};
    use crate::summary::{Concentration, PortfolioTotals, UnderlyingRisk};

    fn config() -> LimitsConfig {
        LimitsConfig {
            warning_clear_threshold: 0.75,
            warning_threshold: 0.8,
            clear_threshold: 0.95,
            portfolio: PortfolioLimits {
                dollar_delta: 100_000.0,
                dollar_gamma_1pct: 10_000.0,
                vega: 5_000.0,
                theta_floor: 2_000.0,
                gross_exposure: 200_000.0,
                max_concentration: 0.6,
            },
            underlying_dollar_delta: UnderlyingDeltaLimit {
                default: 50_000.0,
                overrides: BTreeMap::from([(Symbol::new("NVDA").unwrap(), 20_000.0)]),
            },
            var_99: None,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-09-28T14:30:00Z".parse().unwrap()
    }

    fn monitor() -> LimitMonitor {
        LimitMonitor::new(config(), Box::new(FixedClock::new(now())))
    }

    fn summary_with_dollar_delta(value: f64) -> RiskSummary {
        RiskSummary {
            by_underlying: BTreeMap::new(),
            totals: PortfolioTotals {
                dollar_delta: value,
                ..PortfolioTotals::default()
            },
            concentration: Vec::new(),
            active_positions: 0,
            expired_positions: 0,
            error_positions: 0,
        }
    }

    #[test]
    fn state_transitions_ok_to_warning_to_breach_and_back() {
        let mut monitor = monitor();
        let key = |name, scope: &Scope| (name, scope.clone());
        let scope = Scope::Portfolio;

        let events = monitor.evaluate(&summary_with_dollar_delta(50_000.0), 0.0, now());
        assert!(events.is_empty(), "50 % utilisation should stay Ok");

        let events = monitor.evaluate(&summary_with_dollar_delta(85_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, LimitState::Ok);
        assert_eq!(events[0].to, LimitState::Warning);
        assert_eq!(events[0].name, LimitKind::PortfolioDollarDelta);

        let events = monitor.evaluate(&summary_with_dollar_delta(120_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, LimitState::Warning);
        assert_eq!(events[0].to, LimitState::Breach);

        // Drops below the clear threshold in one step, straight past Warning to Ok.
        let events = monitor.evaluate(&summary_with_dollar_delta(50_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, LimitState::Breach);
        assert_eq!(events[0].to, LimitState::Ok);

        assert!(
            !monitor
                .active()
                .any(|s| key(s.name, &s.scope) == key(LimitKind::PortfolioDollarDelta, &scope))
        );
    }

    #[test]
    fn hysteresis_keeps_one_breach_oscillating_around_the_line() {
        let mut monitor = monitor();

        // Establish a baseline Ok state first.
        let events = monitor.evaluate(&summary_with_dollar_delta(10_000.0), 0.0, now());
        assert!(events.is_empty());

        // Cross into breach.
        let events = monitor.evaluate(&summary_with_dollar_delta(101_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to, LimitState::Breach);

        // Oscillate between 0.97x and 1.01x the limit several times: 0.97 is above the
        // 0.95 clear threshold, so the breach should never clear.
        for _ in 0..5 {
            let events = monitor.evaluate(&summary_with_dollar_delta(97_000.0), 0.0, now());
            assert!(events.is_empty(), "0.97x should not clear the breach");
            let events = monitor.evaluate(&summary_with_dollar_delta(101_000.0), 0.0, now());
            assert!(events.is_empty(), "already in breach, no new event");
        }

        assert_eq!(
            monitor.history().count(),
            1,
            "only the initial breach event"
        );
    }

    #[test]
    fn hysteresis_keeps_one_warning_event_oscillating_around_the_warning_line() {
        let mut monitor = monitor();

        // Establish a baseline Ok state first.
        let events = monitor.evaluate(&summary_with_dollar_delta(10_000.0), 0.0, now());
        assert!(events.is_empty());

        // Cross into warning.
        let events = monitor.evaluate(&summary_with_dollar_delta(82_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to, LimitState::Warning);

        // Oscillate between 0.77x and 0.82x the limit several times: 0.77 is above the
        // 0.75 warning clear threshold, so the warning should never clear back to Ok.
        for _ in 0..5 {
            let events = monitor.evaluate(&summary_with_dollar_delta(77_000.0), 0.0, now());
            assert!(events.is_empty(), "0.77x should not clear the warning");
            let events = monitor.evaluate(&summary_with_dollar_delta(82_000.0), 0.0, now());
            assert!(events.is_empty(), "already in warning, no new event");
        }

        assert_eq!(
            monitor.history().count(),
            1,
            "only the initial warning event"
        );
    }

    #[test]
    fn no_duplicate_events_when_state_stays_the_same() {
        let mut monitor = monitor();
        let events = monitor.evaluate(&summary_with_dollar_delta(85_000.0), 0.0, now());
        assert_eq!(events.len(), 1);
        let events = monitor.evaluate(&summary_with_dollar_delta(86_000.0), 0.0, now());
        assert!(events.is_empty(), "still Warning, no new event");
    }

    #[test]
    fn per_symbol_override_beats_the_default() {
        let mut monitor = monitor();
        let nvda = Symbol::new("NVDA").unwrap();
        let aapl = Symbol::new("AAPL").unwrap();
        let summary = RiskSummary {
            by_underlying: BTreeMap::from([
                (
                    nvda.clone(),
                    UnderlyingRisk {
                        dollar_delta: 19_000.0,
                        ..UnderlyingRisk::default()
                    },
                ),
                (
                    aapl.clone(),
                    UnderlyingRisk {
                        dollar_delta: 19_000.0,
                        ..UnderlyingRisk::default()
                    },
                ),
            ]),
            totals: PortfolioTotals::default(),
            concentration: Vec::new(),
            active_positions: 2,
            expired_positions: 0,
            error_positions: 0,
        };
        monitor.evaluate(&summary, 0.0, now());

        let nvda_status = monitor
            .active()
            .find(|s| s.scope == Scope::Underlying(nvda.clone()))
            .expect("NVDA should be in Warning: 19,000 / 20,000 override = 95 %");
        assert_relative_eq!(nvda_status.limit, 20_000.0);

        assert!(
            monitor
                .active()
                .find(|s| s.scope == Scope::Underlying(aapl.clone()))
                .is_none(),
            "AAPL uses the 50,000 default, so 19,000 stays Ok"
        );
    }

    #[test]
    fn history_is_capped_at_200() {
        let mut monitor = monitor();
        for i in 0..250 {
            let value = if i % 2 == 0 { 90_000.0 } else { 10_000.0 };
            monitor.evaluate(&summary_with_dollar_delta(value), 0.0, now());
        }
        assert_eq!(monitor.history().count(), 200);
    }

    #[test]
    fn theta_floor_ignores_positive_theta() {
        let mut monitor = monitor();
        let summary = RiskSummary {
            by_underlying: BTreeMap::new(),
            totals: PortfolioTotals {
                theta: 10_000.0,
                ..PortfolioTotals::default()
            },
            concentration: Vec::new(),
            active_positions: 0,
            expired_positions: 0,
            error_positions: 0,
        };
        let events = monitor.evaluate(&summary, 0.0, now());
        assert!(
            events.is_empty(),
            "positive theta earns the book money, so it should never breach the floor"
        );
    }

    #[test]
    fn concentration_uses_the_largest_share() {
        let mut monitor = monitor();
        let summary = RiskSummary {
            by_underlying: BTreeMap::new(),
            totals: PortfolioTotals::default(),
            concentration: vec![Concentration {
                symbol: Symbol::new("SPY").unwrap(),
                gross_exposure: 65_000.0,
                fraction: 0.65,
            }],
            active_positions: 0,
            expired_positions: 0,
            error_positions: 0,
        };
        let events = monitor.evaluate(&summary, 0.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, LimitKind::Concentration);
        assert_eq!(events[0].to, LimitState::Breach);
    }

    #[test]
    fn statuses_includes_every_limit_even_ones_still_ok() {
        let monitor = monitor();
        let statuses = monitor.statuses(&summary_with_dollar_delta(50_000.0), 0.0);
        // Six portfolio limits and no underlyings in this summary.
        assert_eq!(statuses.len(), 6);
        assert!(
            statuses.iter().all(|s| s.state == LimitState::Ok),
            "nothing here should have left Ok"
        );
        let delta = statuses
            .iter()
            .find(|s| s.name == LimitKind::PortfolioDollarDelta)
            .expect("portfolio dollar delta should be present even though it's Ok");
        assert_relative_eq!(delta.utilisation, 0.5);
    }

    #[test]
    fn statuses_reflects_hysteresis_state_after_evaluate() {
        let mut monitor = monitor();
        monitor.evaluate(&summary_with_dollar_delta(101_000.0), 0.0, now());
        // Dropping back under the raw breach line does not clear hysteresis, so
        // `statuses` (called after `evaluate`) should still show `Breach`, matching
        // what `active()` shows, not a bare recomputed `value.abs() / limit`.
        let statuses = monitor.statuses(&summary_with_dollar_delta(97_000.0), 0.0);
        let delta = statuses
            .iter()
            .find(|s| s.name == LimitKind::PortfolioDollarDelta)
            .expect("portfolio dollar delta should be present");
        assert_eq!(delta.state, LimitState::Breach);
    }

    #[test]
    fn var_99_limit_is_evaluated_when_configured() {
        let mut cfg = config();
        cfg.var_99 = Some(100_000.0);
        let mut monitor = LimitMonitor::new(cfg, Box::new(FixedClock::new(now())));

        // Below the limit (and below the 0.8 warning threshold): no event.
        let events = monitor.evaluate(&summary_with_dollar_delta(0.0), 50_000.0, now());
        assert!(events.is_empty());

        // Above the limit: a breach event, scoped to the whole portfolio.
        let events = monitor.evaluate(&summary_with_dollar_delta(0.0), 120_000.0, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, LimitKind::PortfolioVar99);
        assert_eq!(events[0].scope, Scope::Portfolio);
        assert_eq!(events[0].to, LimitState::Breach);
    }

    #[test]
    fn var_99_limit_is_absent_from_statuses_when_not_configured() {
        let monitor = monitor();
        let statuses = monitor.statuses(&summary_with_dollar_delta(0.0), 1_000_000.0);
        assert!(!statuses.iter().any(|s| s.name == LimitKind::PortfolioVar99));
    }
}
