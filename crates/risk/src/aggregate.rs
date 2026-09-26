//! The aggregation function.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use domain::Symbol;
use portfolio::{PositionRisk, PositionStatus};

use crate::summary::{Concentration, PortfolioTotals, RiskSummary, UnderlyingRisk};

/// Aggregates per-position valuations into per-underlying risk, portfolio totals and concentration.
///
/// Only active positions add to risk, exposure, market value and unrealised
/// P&L. Expired positions add to the expired count and to their underlying's
/// realised P&L. Error positions add to the error count only, since they carry
/// no numbers. Realised P&L of every row is summed, and it is zero for active
/// and error rows.
///
/// For each active position, with spot S, quantity q and multiplier m:
///
/// - gross exposure adds |dollar delta|, so a long and a short leg both count;
/// - contract notional adds |q × m × S|, which is |q × S| for stock (m = 1);
/// - every other measure adds the position's reporting figure.
///
/// Portfolio totals are the sums of the per-underlying figures. Share delta and
/// share gamma stop at the underlying. Concentration is each underlying's
/// gross exposure divided by the portfolio's, and is empty when that total is
/// zero.
///
/// ```
/// let summary = risk::aggregate(&[]);
/// assert!(summary.by_underlying.is_empty());
/// assert!(summary.concentration.is_empty());
/// assert_eq!(summary.totals.gross_exposure, 0.0);
/// ```
pub fn aggregate(positions: &[PositionRisk]) -> RiskSummary {
    let mut summary = RiskSummary::default();
    for row in positions {
        match row.status {
            PositionStatus::Active => {
                summary.active_positions += 1;
                add_active(entry(&mut summary.by_underlying, &row.underlying), row);
            }
            PositionStatus::Expired => {
                summary.expired_positions += 1;
                let risk = entry(&mut summary.by_underlying, &row.underlying);
                // Show the settlement spot only when no active position has set one.
                if risk.position_count == 0 {
                    risk.spot = row.spot;
                }
                risk.realised_pnl += row.realised_pnl;
            }
            PositionStatus::Error { .. } => summary.error_positions += 1,
        }
    }
    summary.totals = totals(&summary.by_underlying);
    summary.concentration = concentration(&summary.by_underlying, summary.totals.gross_exposure);
    summary
}

fn entry<'a>(
    by_underlying: &'a mut BTreeMap<Symbol, UnderlyingRisk>,
    symbol: &Symbol,
) -> &'a mut UnderlyingRisk {
    by_underlying.entry(symbol.clone()).or_default()
}

fn add_active(risk: &mut UnderlyingRisk, row: &PositionRisk) {
    let g = &row.reporting;
    risk.spot = row.spot;
    risk.share_delta += g.share_delta;
    risk.dollar_delta += g.dollar_delta;
    risk.share_gamma += g.share_gamma;
    risk.dollar_gamma_1pct += g.dollar_gamma_1pct;
    risk.vega += g.vega;
    risk.theta += g.theta;
    risk.rho += g.rho;
    risk.market_value += row.market_value;
    risk.unrealised_pnl += row.unrealised_pnl;
    risk.realised_pnl += row.realised_pnl;
    risk.gross_exposure += g.dollar_delta.abs();
    risk.contract_notional += (row.quantity * row.multiplier * row.spot).abs();
    risk.position_count += 1;
}

/// Sums the dollar measures. Share measures are left out on purpose.
fn totals(by_underlying: &BTreeMap<Symbol, UnderlyingRisk>) -> PortfolioTotals {
    by_underlying
        .values()
        .fold(PortfolioTotals::default(), |mut t, u| {
            t.dollar_delta += u.dollar_delta;
            t.dollar_gamma_1pct += u.dollar_gamma_1pct;
            t.vega += u.vega;
            t.theta += u.theta;
            t.rho += u.rho;
            t.market_value += u.market_value;
            t.unrealised_pnl += u.unrealised_pnl;
            t.realised_pnl += u.realised_pnl;
            t.gross_exposure += u.gross_exposure;
            t.contract_notional += u.contract_notional;
            t
        })
}

fn concentration(
    by_underlying: &BTreeMap<Symbol, UnderlyingRisk>,
    total_gross: f64,
) -> Vec<Concentration> {
    // `!(x > 0.0)` also rejects NaN, so the division below is always safe.
    if !(total_gross > 0.0 && total_gross.is_finite()) {
        return Vec::new();
    }
    let mut list: Vec<Concentration> = by_underlying
        .iter()
        .map(|(symbol, u)| Concentration {
            symbol: symbol.clone(),
            gross_exposure: u.gross_exposure,
            fraction: u.gross_exposure / total_gross,
        })
        .collect();
    // The map iterates in symbol order and the sort is stable, so ties stay in symbol order.
    list.sort_by(|a, b| {
        b.fraction
            .partial_cmp(&a.fraction)
            .unwrap_or(Ordering::Equal)
    });
    list
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;
    use domain::{InstrumentKind, PositionId};
    use portfolio::ReportingGreeks;
    use pricing::Greeks;
    use proptest::prelude::*;

    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    /// Raw per-unit Greeks with the given price, delta, gamma, vega, theta and rho.
    fn raw(price: f64, delta: f64, gamma: f64, vega: f64, theta: f64, rho: f64) -> Greeks {
        Greeks {
            price,
            delta,
            gamma,
            vega,
            theta,
            rho,
            vanna: 0.0,
            volga: 0.0,
        }
    }

    /// An active row built the way `portfolio` builds one.
    fn active(
        id: &str,
        symbol: &str,
        kind: InstrumentKind,
        quantity: f64,
        spot: f64,
        entry: f64,
        greeks: Greeks,
    ) -> PositionRisk {
        let multiplier = if kind == InstrumentKind::Equity {
            1.0
        } else {
            100.0
        };
        let size = quantity * multiplier;
        PositionRisk {
            id: PositionId::new(id).unwrap(),
            description: id.to_owned(),
            underlying: sym(symbol),
            kind,
            quantity,
            multiplier,
            spot,
            vol: 0.0,
            time_to_expiry: 0.0,
            mark: greeks.price,
            market_value: greeks.price * size,
            entry_price: entry,
            unrealised_pnl: (greeks.price - entry) * size,
            realised_pnl: 0.0,
            greeks,
            reporting: ReportingGreeks::scale(&greeks, quantity, multiplier, spot),
            status: PositionStatus::Active,
        }
    }

    fn stock(id: &str, symbol: &str, quantity: f64, spot: f64, entry: f64) -> PositionRisk {
        let mut greeks = raw(spot, 1.0, 0.0, 0.0, 0.0, 0.0);
        greeks.price = spot;
        active(
            id,
            symbol,
            InstrumentKind::Equity,
            quantity,
            spot,
            entry,
            greeks,
        )
    }

    fn expired(id: &str, symbol: &str, spot: f64, realised: f64) -> PositionRisk {
        let mut row = active(
            id,
            symbol,
            InstrumentKind::Call,
            1.0,
            spot,
            0.0,
            raw(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        );
        row.market_value = 0.0;
        row.unrealised_pnl = 0.0;
        row.realised_pnl = realised;
        row.status = PositionStatus::Expired;
        row
    }

    fn errored(id: &str, symbol: &str) -> PositionRisk {
        let mut row = active(
            id,
            symbol,
            InstrumentKind::Call,
            1.0,
            0.0,
            0.0,
            raw(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        );
        row.status = PositionStatus::Error {
            reason: "no market data".to_owned(),
        };
        row
    }

    /// Two underlyings, four positions.
    fn hand_book() -> Vec<PositionRisk> {
        vec![
            stock("aaa-stock", "AAA", 200.0, 100.0, 90.0),
            // Short 2 calls: size = -2 × 100 = -200.
            active(
                "aaa-call",
                "AAA",
                InstrumentKind::Call,
                -2.0,
                100.0,
                4.0,
                raw(5.0, 0.5, 0.02, 20.0, -36.5, 25.0),
            ),
            // Long 3 puts: size = 3 × 100 = 300.
            active(
                "bbb-put",
                "BBB",
                InstrumentKind::Put,
                3.0,
                50.0,
                2.5,
                raw(2.0, -0.4, 0.05, 10.0, -18.25, -12.0),
            ),
            stock("bbb-stock", "BBB", -100.0, 50.0, 55.0),
        ]
    }

    #[test]
    fn hand_computed_two_underlyings() {
        let summary = aggregate(&hand_book());

        // AAA, spot 100.
        //   stock 200:     share delta 200, dollar delta 200 × 100 = 20,000,
        //                  market value 100 × 200 = 20,000, unrealised (100 - 90) × 200 = 2,000.
        //   short 2 calls: size -200. Share delta 0.5 × -200 = -100, dollar delta -10,000,
        //                  share gamma 0.02 × -200 = -4, dollar gamma ½ × 0.02 × 1² × -200 = -2,
        //                  vega 20 / 100 × -200 = -40, theta -36.5 / 365 × -200 = +20,
        //                  rho 25 / 100 × -200 = -50, market value 5 × -200 = -1,000,
        //                  unrealised (5 - 4) × -200 = -200.
        //   Sums: share delta 100, dollar delta 10,000, market value 19,000, unrealised 1,800.
        //   Gross exposure 20,000 + 10,000 = 30,000.
        //   Contract notional 200 × 100 + 200 × 100 = 40,000.
        let a = &summary.by_underlying[&sym("AAA")];
        assert_abs_diff_eq!(a.spot, 100.0);
        assert_abs_diff_eq!(a.share_delta, 100.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.dollar_delta, 10_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.share_gamma, -4.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.dollar_gamma_1pct, -2.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.vega, -40.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.theta, 20.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.rho, -50.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.market_value, 19_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.unrealised_pnl, 1_800.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.gross_exposure, 30_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.contract_notional, 40_000.0, epsilon = 1e-9);
        assert_eq!(a.position_count, 2);

        // BBB, spot 50.
        //   long 3 puts: size 300. Share delta -0.4 × 300 = -120, dollar delta -6,000,
        //                share gamma 0.05 × 300 = 15,
        //                dollar gamma ½ × 0.05 × 0.5² × 300 = 1.875,
        //                vega 10 / 100 × 300 = 30, theta -18.25 / 365 × 300 = -15,
        //                rho -12 / 100 × 300 = -36, market value 2 × 300 = 600,
        //                unrealised (2 - 2.5) × 300 = -150.
        //   short stock 100: share delta -100, dollar delta -5,000,
        //                market value 50 × -100 = -5,000, unrealised (50 - 55) × -100 = 500.
        //   Sums: share delta -220, dollar delta -11,000, market value -4,400, unrealised 350.
        //   Gross exposure 6,000 + 5,000 = 11,000.
        //   Contract notional 300 × 50 + 100 × 50 = 20,000.
        let b = &summary.by_underlying[&sym("BBB")];
        assert_abs_diff_eq!(b.share_delta, -220.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.dollar_delta, -11_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.share_gamma, 15.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.dollar_gamma_1pct, 1.875, epsilon = 1e-9);
        assert_abs_diff_eq!(b.vega, 30.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.theta, -15.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.rho, -36.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.market_value, -4_400.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.unrealised_pnl, 350.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.gross_exposure, 11_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(b.contract_notional, 20_000.0, epsilon = 1e-9);

        // Portfolio: dollar delta 10,000 - 11,000 = -1,000, dollar gamma -2 + 1.875 = -0.125,
        // vega -40 + 30 = -10, theta 20 - 15 = 5, rho -50 - 36 = -86,
        // market value 19,000 - 4,400 = 14,600, unrealised 1,800 + 350 = 2,150,
        // gross exposure 30,000 + 11,000 = 41,000, notional 40,000 + 20,000 = 60,000.
        let t = &summary.totals;
        assert_abs_diff_eq!(t.dollar_delta, -1_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.dollar_gamma_1pct, -0.125, epsilon = 1e-9);
        assert_abs_diff_eq!(t.vega, -10.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.theta, 5.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.rho, -86.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.market_value, 14_600.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.unrealised_pnl, 2_150.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.realised_pnl, 0.0);
        assert_abs_diff_eq!(t.gross_exposure, 41_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(t.contract_notional, 60_000.0, epsilon = 1e-9);

        // Concentration: 30,000 / 41,000 and 11,000 / 41,000, largest first.
        let c = &summary.concentration;
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].symbol, sym("AAA"));
        assert_abs_diff_eq!(c[0].fraction, 30_000.0 / 41_000.0, epsilon = 1e-12);
        assert_eq!(c[1].symbol, sym("BBB"));
        assert_abs_diff_eq!(c[1].fraction, 11_000.0 / 41_000.0, epsilon = 1e-12);

        assert_eq!(summary.active_positions, 4);
        assert_eq!(summary.expired_positions, 0);
        assert_eq!(summary.error_positions, 0);
    }

    #[test]
    fn covered_call_nets_to_seventy_shares() {
        // 100 shares and one short call of delta 0.30: 100 - 0.30 × 100 = 70 shares.
        let rows = [
            stock("cc-stock", "AAPL", 100.0, 200.0, 200.0),
            active(
                "cc-call",
                "AAPL",
                InstrumentKind::Call,
                -1.0,
                200.0,
                4.0,
                raw(4.0, 0.3, 0.02, 30.0, -20.0, 10.0),
            ),
        ];
        let summary = aggregate(&rows);
        let a = &summary.by_underlying[&sym("AAPL")];
        assert_abs_diff_eq!(a.share_delta, 70.0, epsilon = 1e-9);
        // Net dollar delta 70 × 200 = 14,000, but gross exposure is 100 × 200 + 30 × 200 = 26,000.
        assert_abs_diff_eq!(a.dollar_delta, 14_000.0, epsilon = 1e-9);
        assert_abs_diff_eq!(a.gross_exposure, 26_000.0, epsilon = 1e-9);
        let legs: f64 = rows.iter().map(|r| r.reporting.dollar_delta.abs()).sum();
        assert_abs_diff_eq!(a.gross_exposure, legs, epsilon = 1e-9);
        // Notional counts the option at its underlying value: 100 × 200 + 100 × 200.
        assert_abs_diff_eq!(a.contract_notional, 40_000.0, epsilon = 1e-9);
    }

    #[test]
    fn opposite_legs_cancel_in_delta_but_not_in_exposure() {
        let rows = [
            stock("long", "XYZ", 100.0, 10.0, 10.0),
            stock("short", "XYZ", -100.0, 10.0, 10.0),
        ];
        let summary = aggregate(&rows);
        assert_abs_diff_eq!(summary.totals.dollar_delta, 0.0);
        assert_abs_diff_eq!(summary.totals.gross_exposure, 2_000.0);
        assert_abs_diff_eq!(summary.totals.contract_notional, 2_000.0);
        // Exposure is positive, so the single underlying holds all of it.
        assert_abs_diff_eq!(summary.concentration[0].fraction, 1.0);
    }

    #[test]
    fn expired_book_has_zero_risk_and_realised_pnl() {
        let rows = [
            expired("e1", "SPY", 601.0, 250.0),
            expired("e2", "SPY", 601.0, -100.0),
            expired("e3", "AAPL", 230.0, 40.0),
        ];
        let summary = aggregate(&rows);
        let t = &summary.totals;
        assert_abs_diff_eq!(t.realised_pnl, 190.0);
        for measure in [
            t.dollar_delta,
            t.dollar_gamma_1pct,
            t.vega,
            t.theta,
            t.rho,
            t.market_value,
            t.unrealised_pnl,
            t.gross_exposure,
            t.contract_notional,
        ] {
            assert_abs_diff_eq!(measure, 0.0);
        }
        assert_eq!(summary.expired_positions, 3);
        assert_eq!(summary.active_positions, 0);
        assert!(summary.concentration.is_empty());
        // The underlying still shows, with the settlement spot and its realised P&L.
        let spy = &summary.by_underlying[&sym("SPY")];
        assert_abs_diff_eq!(spy.spot, 601.0);
        assert_abs_diff_eq!(spy.realised_pnl, 150.0);
        assert_eq!(spy.position_count, 0);
    }

    #[test]
    fn expired_and_error_rows_add_to_counts_but_not_risk() {
        let mut rows = hand_book();
        let before = aggregate(&rows);
        rows.push(expired("done", "AAA", 100.0, 75.0));
        rows.push(errored("bad", "CCC"));
        let after = aggregate(&rows);

        assert_eq!(after.active_positions, 4);
        assert_eq!(after.expired_positions, 1);
        assert_eq!(after.error_positions, 1);
        assert_abs_diff_eq!(after.totals.realised_pnl, 75.0);
        assert_abs_diff_eq!(after.totals.gross_exposure, before.totals.gross_exposure);
        assert_abs_diff_eq!(after.totals.dollar_delta, before.totals.dollar_delta);
        // The expired row does not change AAA's spot or count, and the error row makes no entry.
        assert_eq!(after.by_underlying[&sym("AAA")].position_count, 2);
        assert_abs_diff_eq!(after.by_underlying[&sym("AAA")].spot, 100.0);
        assert!(!after.by_underlying.contains_key(&sym("CCC")));
    }

    #[test]
    fn active_spot_wins_whatever_the_row_order() {
        let mut rows = vec![
            expired("done", "AAA", 99.0, 0.0),
            stock("live", "AAA", 1.0, 100.0, 100.0),
        ];
        assert_abs_diff_eq!(aggregate(&rows).by_underlying[&sym("AAA")].spot, 100.0);
        rows.reverse();
        assert_abs_diff_eq!(aggregate(&rows).by_underlying[&sym("AAA")].spot, 100.0);
    }

    #[test]
    fn concentration_is_empty_when_gross_exposure_is_zero() {
        // An active position with no delta and no spot exposure has zero gross exposure.
        let flat = active(
            "flat",
            "AAA",
            InstrumentKind::Call,
            1.0,
            100.0,
            1.0,
            raw(1.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        );
        let summary = aggregate(&[flat]);
        assert_abs_diff_eq!(summary.totals.gross_exposure, 0.0);
        assert!(summary.concentration.is_empty());
        assert!(aggregate(&[]).concentration.is_empty());
    }

    #[test]
    fn zero_exposure_underlying_is_listed_with_zero_fraction() {
        let flat = active(
            "flat",
            "AAA",
            InstrumentKind::Call,
            1.0,
            100.0,
            1.0,
            raw(1.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        );
        let summary = aggregate(&[flat, stock("s", "BBB", 10.0, 5.0, 5.0)]);
        let c = &summary.concentration;
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].symbol, sym("BBB"));
        assert_abs_diff_eq!(c[0].fraction, 1.0);
        assert_abs_diff_eq!(c[1].fraction, 0.0);
    }

    #[test]
    fn concentration_ties_keep_symbol_order() {
        let rows = [
            stock("b", "BBB", 10.0, 10.0, 10.0),
            stock("a", "AAA", 10.0, 10.0, 10.0),
        ];
        let c = aggregate(&rows).concentration;
        assert_eq!(c[0].symbol, sym("AAA"));
        assert_eq!(c[1].symbol, sym("BBB"));
    }

    #[test]
    fn serialises_with_snake_case_fields_and_no_share_measures_in_totals() {
        let json = serde_json::to_value(aggregate(&hand_book())).unwrap();
        let object = json.as_object().unwrap();
        for key in [
            "by_underlying",
            "totals",
            "concentration",
            "active_positions",
            "expired_positions",
            "error_positions",
        ] {
            assert!(object.contains_key(key), "missing {key}");
        }
        let aaa = json["by_underlying"]["AAA"].as_object().unwrap();
        for key in [
            "spot",
            "share_delta",
            "dollar_delta",
            "share_gamma",
            "dollar_gamma_1pct",
            "vega",
            "theta",
            "rho",
            "market_value",
            "unrealised_pnl",
            "realised_pnl",
            "gross_exposure",
            "contract_notional",
            "position_count",
        ] {
            assert!(aaa.contains_key(key), "missing {key}");
        }
        let totals = json["totals"].as_object().unwrap();
        assert!(!totals.contains_key("share_delta"));
        assert!(!totals.contains_key("share_gamma"));
        assert_eq!(totals.len(), 10);
        assert_eq!(json["concentration"][0]["symbol"], "AAA");
    }

    /// A random row: 0 to 2 pick the status, and the rest are its inputs.
    fn arb_row() -> impl Strategy<Value = PositionRisk> {
        (
            0..3_u8,
            0..4_usize,
            prop_oneof![
                Just(InstrumentKind::Equity),
                Just(InstrumentKind::Call),
                Just(InstrumentKind::Put)
            ],
            -1_000.0..1_000.0_f64,
            1.0..1_000.0_f64,
            (
                0.0..50.0_f64,
                -1.0..1.0_f64,
                0.0..0.1_f64,
                0.0..100.0_f64,
                -100.0..0.0_f64,
                -100.0..100.0_f64,
            ),
            -500.0..500.0_f64,
        )
            .prop_map(|(status, sym_index, kind, quantity, spot, g, realised)| {
                let symbol = ["AAA", "BBB", "CCC", "DDD"][sym_index];
                let greeks = if kind == InstrumentKind::Equity {
                    raw(spot, 1.0, 0.0, 0.0, 0.0, 0.0)
                } else {
                    raw(g.0, g.1, g.2, g.3, g.4, g.5)
                };
                let mut row = active("p", symbol, kind, quantity, spot, 1.0, greeks);
                match status {
                    0 => {}
                    1 => {
                        row.market_value = 0.0;
                        row.unrealised_pnl = 0.0;
                        row.reporting = ReportingGreeks::default();
                        row.realised_pnl = realised;
                        row.status = PositionStatus::Expired;
                    }
                    _ => {
                        row = errored("p", symbol);
                    }
                }
                row
            })
    }

    proptest! {
        #[test]
        fn totals_equal_sums_over_underlyings_and_positions(
            rows in prop::collection::vec(arb_row(), 0..40)
        ) {
            let summary = aggregate(&rows);
            let live: Vec<&PositionRisk> = rows
                .iter()
                .filter(|r| r.status == PositionStatus::Active)
                .collect();

            // Compares a total with a sum, allowing for the different order of addition.
            let close = |total: f64, parts: &[f64]| {
                let scale: f64 = parts.iter().map(|x| x.abs()).sum();
                (total - parts.iter().sum::<f64>()).abs() <= 1e-9 * (1.0 + scale)
            };
            let by_u = |f: fn(&UnderlyingRisk) -> f64| -> Vec<f64> {
                summary.by_underlying.values().map(f).collect()
            };
            let by_p = |f: fn(&PositionRisk) -> f64| -> Vec<f64> {
                live.iter().map(|r| f(r)).collect()
            };
            let t = &summary.totals;

            prop_assert!(close(t.dollar_delta, &by_u(|u| u.dollar_delta)));
            prop_assert!(close(t.dollar_delta, &by_p(|r| r.reporting.dollar_delta)));
            prop_assert!(close(t.dollar_gamma_1pct, &by_u(|u| u.dollar_gamma_1pct)));
            prop_assert!(close(t.dollar_gamma_1pct, &by_p(|r| r.reporting.dollar_gamma_1pct)));
            prop_assert!(close(t.vega, &by_u(|u| u.vega)));
            prop_assert!(close(t.vega, &by_p(|r| r.reporting.vega)));
            prop_assert!(close(t.theta, &by_p(|r| r.reporting.theta)));
            prop_assert!(close(t.rho, &by_p(|r| r.reporting.rho)));
            prop_assert!(close(t.market_value, &by_p(|r| r.market_value)));
            prop_assert!(close(t.unrealised_pnl, &by_p(|r| r.unrealised_pnl)));
            prop_assert!(close(t.gross_exposure, &by_u(|u| u.gross_exposure)));
            prop_assert!(close(t.gross_exposure, &by_p(|r| r.reporting.dollar_delta.abs())));
            prop_assert!(close(
                t.contract_notional,
                &by_p(|r| (r.quantity * r.multiplier * r.spot).abs())
            ));
            let all: Vec<f64> = rows.iter().map(|r| r.realised_pnl).collect();
            prop_assert!(close(t.realised_pnl, &all));

            // Gross exposure bounds net dollar delta, per underlying and overall.
            for u in summary.by_underlying.values() {
                prop_assert!(u.gross_exposure + 1e-9 * (1.0 + u.gross_exposure) >= u.dollar_delta.abs());
            }
            prop_assert!(t.gross_exposure + 1e-9 * (1.0 + t.gross_exposure) >= t.dollar_delta.abs());

            // Counts add up to the number of rows.
            prop_assert_eq!(
                summary.active_positions + summary.expired_positions + summary.error_positions,
                rows.len()
            );
            prop_assert_eq!(summary.active_positions, live.len());
        }

        #[test]
        fn concentration_sums_to_one_or_is_empty(
            rows in prop::collection::vec(arb_row(), 0..40)
        ) {
            let summary = aggregate(&rows);
            let c = &summary.concentration;
            if summary.totals.gross_exposure > 0.0 {
                prop_assert_eq!(c.len(), summary.by_underlying.len());
                let sum: f64 = c.iter().map(|x| x.fraction).sum();
                prop_assert!((sum - 1.0).abs() < 1e-9, "sum {}", sum);
                prop_assert!(c.iter().all(|x| (0.0..=1.0).contains(&x.fraction)));
                prop_assert!(c.windows(2).all(|w| w[0].fraction >= w[1].fraction));
            } else {
                prop_assert!(c.is_empty());
            }
        }
    }
}
