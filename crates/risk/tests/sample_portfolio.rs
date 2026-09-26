//! The committed sample portfolio, valued by `portfolio` and aggregated by `risk`.

// Wrapped in `cfg(test)` so the workspace's allow-unwrap-in-tests setting covers the helpers too.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use approx::assert_abs_diff_eq;
    use chrono::{DateTime, TimeZone, Utc};
    use domain::{MarketState, Symbol, UnderlyingState};
    use portfolio::{PositionBook, load_csv, resolve};
    use risk::aggregate;

    fn market(as_of: DateTime<Utc>) -> MarketState {
        let u = |spot, vol| UnderlyingState::new(spot, vol, 0.005).unwrap();
        let entries = [
            ("SPY", u(600.0, 0.16)),
            ("AAPL", u(230.0, 0.26)),
            ("MSFT", u(420.0, 0.24)),
            ("NVDA", u(135.0, 0.50)),
            ("TSLA", u(250.0, 0.60)),
        ];
        MarketState::new(
            as_of,
            0.04,
            entries.map(|(s, state)| (Symbol::new(s).unwrap(), state)),
        )
        .unwrap()
    }

    fn book_at(now: DateTime<Utc>) -> (PositionBook, MarketState) {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/sample_portfolio.csv");
        let m = market(now);
        let positions = resolve(&load_csv(&path).unwrap(), &m, now).unwrap();
        (PositionBook::from_positions(positions).unwrap(), m)
    }

    #[test]
    fn sample_book_aggregates_consistently() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
        let (book, m) = book_at(now);
        let rows = book.value_all(&m, now);
        let summary = aggregate(&rows);

        assert_eq!(summary.active_positions, 12);
        assert_eq!(summary.expired_positions, 0);
        assert_eq!(summary.error_positions, 0);
        assert_eq!(summary.by_underlying.len(), 5);
        assert_abs_diff_eq!(
            summary.totals.dollar_delta,
            rows.iter().map(|r| r.reporting.dollar_delta).sum::<f64>(),
            epsilon = 1e-6
        );
        // Blank entry prices mean the book starts at zero unrealised P&L.
        assert_abs_diff_eq!(summary.totals.unrealised_pnl, 0.0, epsilon = 1e-6);
        let fractions: f64 = summary.concentration.iter().map(|c| c.fraction).sum();
        assert_abs_diff_eq!(fractions, 1.0, epsilon = 1e-12);

        // Covered call: 300 shares less three short calls of 100 shares each at the call's delta.
        let aapl = &summary.by_underlying[&Symbol::new("AAPL").unwrap()];
        let call = rows
            .iter()
            .find(|r| r.id.as_str() == "aapl-cc-call")
            .unwrap();
        assert_abs_diff_eq!(
            aapl.share_delta,
            300.0 - 3.0 * 100.0 * call.greeks.delta,
            epsilon = 1e-9
        );
        assert!(aapl.share_delta > 0.0 && aapl.share_delta < 300.0);
        assert_abs_diff_eq!(
            aapl.gross_exposure,
            (300.0 * 230.0) + (3.0 * 100.0 * call.greeks.delta * 230.0),
            epsilon = 1e-6
        );
    }

    #[test]
    fn settled_sample_call_moves_to_realised_and_out_of_risk() {
        let start = Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
        let (mut book, m) = book_at(start);
        let before = aggregate(&book.value_all(&m, start));

        let after_close = Utc.with_ymd_and_hms(2026, 10, 1, 20, 0, 0).unwrap();
        book.settle_expired(after_close, &m);
        let after = aggregate(&book.value_all(&m, after_close));

        assert_eq!(after.active_positions, 11);
        assert_eq!(after.expired_positions, 1);
        // The ATM call settles at intrinsic zero against an entry at its theoretical value,
        // so the five contracts lose their premium in realised P&L, and it leaves the risk.
        assert!(after.totals.realised_pnl < 0.0);
        assert!(after.totals.gross_exposure < before.totals.gross_exposure);
    }
}
