//! Split adjustment rules for positions, shared by the live book and by saved specs.
//!
//! The economic rule behind OCC adjustments: a split must leave the value of a holding
//! unchanged. For a stock that is quantity × ratio at entry ÷ ratio. For an option
//! there are two cases.
//!
//! - **Whole-number forward split** (2-for-1, 4-for-1): contracts × ratio, strike ÷ ratio,
//!   multiplier unchanged.
//! - **Any other ratio** (3-for-2, a reverse split such as 1-for-10): contracts unchanged,
//!   multiplier × ratio, strike ÷ ratio.
//!
//! In both cases the per-unit entry price is divided by the ratio, so quantity × multiplier
//! × entry price is unchanged. Black-Scholes prices are homogeneous of degree one in
//! (spot, strike), so with spot and strike both divided by the ratio the option price is
//! divided by the ratio and the position value is unchanged.
//!
//! Strikes are **not rounded**. The OCC rounds an adjusted strike to a listed increment
//! (a 3-for-2 on a 100 strike gives 66.67, and a contract for 150 shares is what is kept).
//! This model keeps the exact ratio, so 66.6667 stays 66.6667.

use domain::{AppliedAction, CorporateAction, Symbol, is_whole_number_forward_split};

/// The contract terms a split changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Terms {
    /// Signed shares or contracts.
    pub quantity: f64,
    /// Entry price per unit.
    pub entry_price: f64,
    /// Option strike. `None` for equity.
    pub strike: Option<f64>,
    /// Units per quantity: 1 for shares, the contract multiplier for options.
    pub multiplier: f64,
}

/// The terms after a split of `ratio`. Options are told apart by having a strike.
pub(crate) fn split_terms(terms: Terms, ratio: f64) -> Terms {
    let entry_price = terms.entry_price / ratio;
    match terms.strike {
        None => Terms {
            quantity: terms.quantity * ratio,
            entry_price,
            ..terms
        },
        Some(strike) if is_whole_number_forward_split(ratio) => Terms {
            quantity: terms.quantity * ratio,
            entry_price,
            strike: Some(strike / ratio),
            multiplier: terms.multiplier,
        },
        Some(strike) => Terms {
            quantity: terms.quantity,
            entry_price,
            strike: Some(strike / ratio),
            multiplier: terms.multiplier * ratio,
        },
    }
}

/// A split already applied, taken from the ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedSplit {
    /// The action id.
    pub id: String,
    /// The underlying that split.
    pub symbol: Symbol,
    /// The split ratio.
    pub ratio: f64,
}

/// The splits in a ledger, in ledger order. Dividends are left out: they change only cash,
/// so they are never re-applied to a book.
pub fn ledger_splits(ledger: &[AppliedAction]) -> Vec<AppliedSplit> {
    ledger
        .iter()
        .filter_map(|applied| match &applied.action {
            CorporateAction::Split { symbol, .. } => Some(AppliedSplit {
                id: applied.id.clone(),
                symbol: symbol.clone(),
                ratio: applied.amount,
            }),
            CorporateAction::CashDividend { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    fn option(quantity: f64) -> Terms {
        Terms {
            quantity,
            entry_price: 8.0,
            strike: Some(200.0),
            multiplier: 100.0,
        }
    }

    #[test]
    fn a_whole_number_forward_split_multiplies_contracts() {
        let t = split_terms(option(3.0), 4.0);
        assert_relative_eq!(t.quantity, 12.0);
        assert_relative_eq!(t.strike.unwrap(), 50.0);
        assert_relative_eq!(t.multiplier, 100.0);
        assert_relative_eq!(t.entry_price, 2.0);
    }

    #[test]
    fn other_ratios_change_the_multiplier() {
        let t = split_terms(option(3.0), 1.5);
        assert_relative_eq!(t.quantity, 3.0);
        assert_relative_eq!(t.multiplier, 150.0);
        // The strike is not rounded: 200 / 1.5 stays 133.333...
        assert_relative_eq!(t.strike.unwrap(), 200.0 / 1.5);
        let reverse = split_terms(option(3.0), 0.1);
        assert_relative_eq!(reverse.multiplier, 10.0);
        assert_relative_eq!(reverse.strike.unwrap(), 2000.0);
        assert_relative_eq!(reverse.quantity, 3.0);
    }

    #[test]
    fn value_at_entry_is_preserved_in_every_case() {
        for ratio in [4.0, 1.5, 0.1, 2.0] {
            let before = option(3.0);
            let after = split_terms(before, ratio);
            assert_relative_eq!(
                before.quantity * before.multiplier * before.entry_price,
                after.quantity * after.multiplier * after.entry_price,
                max_relative = 1e-12
            );
        }
    }

    #[test]
    fn stock_quantity_scales_and_entry_divides() {
        let t = split_terms(
            Terms {
                quantity: 10.0,
                entry_price: 200.0,
                strike: None,
                multiplier: 1.0,
            },
            4.0,
        );
        assert_relative_eq!(t.quantity, 40.0);
        assert_relative_eq!(t.entry_price, 50.0);
        assert_relative_eq!(t.multiplier, 1.0);
    }
}
