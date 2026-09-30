//! The committed demo corporate actions are valid against the committed market config and
//! give the demo its four events: one 4-for-1 split, one 3-for-2 split and two dividends.
#![allow(clippy::unwrap_used)]

use chrono::NaiveDate;
use engine::{
    CorporateAction, CorporateActionCalendar, CorporateActionsConfig, MarketConfig, Symbol,
};

mod support;

#[test]
fn the_demo_events_are_valid_and_always_ahead_of_the_start() {
    let root = support::repo_root();
    let market = MarketConfig::load(&root.join("config/market.toml")).unwrap();
    let config = CorporateActionsConfig::load(&root.join("config/corporate_actions.toml")).unwrap();
    assert!(
        !config.alpha_vantage.enabled,
        "the real-data loader is opt-in"
    );

    let start = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let calendar = CorporateActionCalendar::resolve(&config, start).unwrap();
    calendar.validate_against(&market).unwrap();
    assert_eq!(calendar.pending().len(), 4);

    let splits: Vec<f64> = calendar
        .pending()
        .iter()
        .filter_map(|a| match a.action {
            CorporateAction::Split { ratio, .. } => Some(ratio),
            CorporateAction::CashDividend { .. } => None,
        })
        .collect();
    assert_eq!(splits, [4.0, 1.5]);
    let dividend_symbols: Vec<&Symbol> = calendar
        .pending()
        .iter()
        .filter(|a| matches!(a.action, CorporateAction::CashDividend { .. }))
        .map(|a| a.action.symbol())
        .collect();
    assert_eq!(dividend_symbols.len(), 2);
    assert_ne!(dividend_symbols[0], dividend_symbols[1]);
    assert!(calendar.pending().iter().all(|a| a.action.date() > start));

    // Every dividend symbol is on the discrete model, so it prices with q = 0.
    for symbol in dividend_symbols {
        assert!(market.discrete_symbols().contains(symbol));
    }
}
