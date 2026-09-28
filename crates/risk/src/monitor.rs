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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
}

/// The whole portfolio, or one underlying.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
/// it is `Warning`. Below that it is `Ok`. Hysteresis stops flapping around the breach
/// line: once a limit reaches `Breach`, it stays there until utilisation drops below
/// [`LimitsConfig::clear_threshold`], even if that means sitting in `Breach` at a
/// utilisation that would otherwise read as `Warning` or `Ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitState {
    /// Utilisation is below the warning threshold (or hysteresis has not yet cleared).
    Ok,
    /// Utilisation is at or above the warning threshold and below 1, or a breach has not
    /// yet cleared below the clear threshold.
    Warning,
    /// Utilisation is at or above 1, or a breach has not yet cleared below the clear
    /// threshold.
    Breach,
}

/// The next state given the current one and the fresh utilisation reading.
fn next_state(
    current: LimitState,
    utilisation: f64,
    warning_threshold: f64,
    clear_threshold: f64,
) -> LimitState {
    if current == LimitState::Breach && utilisation >= clear_threshold {
        return LimitState::Breach;
    }
    if utilisation >= 1.0 {
        LimitState::Breach
    } else if utilisation >= warning_threshold {
        LimitState::Warning
    } else {
        LimitState::Ok
    }
}

/// One limit's status for one scope, at one evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// state changed on this call. `sim_time` is stamped on each event; the wall time
    /// comes from this monitor's clock.
    pub fn evaluate(&mut self, summary: &RiskSummary, sim_time: DateTime<Utc>) -> Vec<AlertEvent> {
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
        for (symbol, risk) in &summary.by_underlying {
            let limit = self.config.underlying_dollar_delta.limit_for(symbol);
            statuses.push(self.status_for(
                LimitKind::UnderlyingDollarDelta,
                Scope::Underlying(symbol.clone()),
                risk.dollar_delta,
                limit,
            ));
        }

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

    /// Builds the [`LimitStatus`] for one limit and scope, looking up its current state
    /// for the hysteresis rule.
    fn status_for(&self, name: LimitKind, scope: Scope, value: f64, limit: f64) -> LimitStatus {
        let key = (name, scope.clone());
        let previous = self.states.get(&key).copied().unwrap_or(LimitState::Ok);
        let utilisation = value.abs() / limit;
        let state = next_state(
            previous,
            utilisation,
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

        let events = monitor.evaluate(&summary_with_dollar_delta(50_000.0), now());
        assert!(events.is_empty(), "50 % utilisation should stay Ok");

        let events = monitor.evaluate(&summary_with_dollar_delta(85_000.0), now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, LimitState::Ok);
        assert_eq!(events[0].to, LimitState::Warning);
        assert_eq!(events[0].name, LimitKind::PortfolioDollarDelta);

        let events = monitor.evaluate(&summary_with_dollar_delta(120_000.0), now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, LimitState::Warning);
        assert_eq!(events[0].to, LimitState::Breach);

        // Drops below the clear threshold in one step, straight past Warning to Ok.
        let events = monitor.evaluate(&summary_with_dollar_delta(50_000.0), now());
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
        let events = monitor.evaluate(&summary_with_dollar_delta(10_000.0), now());
        assert!(events.is_empty());

        // Cross into breach.
        let events = monitor.evaluate(&summary_with_dollar_delta(101_000.0), now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to, LimitState::Breach);

        // Oscillate between 0.97x and 1.01x the limit several times: 0.97 is above the
        // 0.95 clear threshold, so the breach should never clear.
        for _ in 0..5 {
            let events = monitor.evaluate(&summary_with_dollar_delta(97_000.0), now());
            assert!(events.is_empty(), "0.97x should not clear the breach");
            let events = monitor.evaluate(&summary_with_dollar_delta(101_000.0), now());
            assert!(events.is_empty(), "already in breach, no new event");
        }

        assert_eq!(
            monitor.history().count(),
            1,
            "only the initial breach event"
        );
    }

    #[test]
    fn no_duplicate_events_when_state_stays_the_same() {
        let mut monitor = monitor();
        let events = monitor.evaluate(&summary_with_dollar_delta(85_000.0), now());
        assert_eq!(events.len(), 1);
        let events = monitor.evaluate(&summary_with_dollar_delta(86_000.0), now());
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
        monitor.evaluate(&summary, now());

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
            monitor.evaluate(&summary_with_dollar_delta(value), now());
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
        let events = monitor.evaluate(&summary, now());
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
        let events = monitor.evaluate(&summary, now());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, LimitKind::Concentration);
        assert_eq!(events[0].to, LimitState::Breach);
    }
}
