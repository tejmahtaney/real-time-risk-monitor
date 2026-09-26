//! The committed sample portfolio loads, resolves and values against a hand-built market.

// Wrapped in `cfg(test)` so the workspace's allow-unwrap-in-tests setting covers the helpers too.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use approx::assert_abs_diff_eq;
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use domain::{Instrument, InstrumentKind, MarketState, Symbol, UnderlyingState};
    use portfolio::{PositionBook, PositionStatus, load_csv, resolve};

    fn sample_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/sample_portfolio.csv")
    }

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

    /// Thursday 1 October 2026, 10:30 New York (14:30 UTC).
    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap()
    }

    #[test]
    fn sample_file_loads_and_resolves() {
        let specs = load_csv(&sample_path()).unwrap();
        assert_eq!(specs.len(), 12);

        let positions = resolve(&specs, &market(start()), start()).unwrap();
        let book = PositionBook::from_positions(positions).unwrap();
        assert_eq!(book.len(), 12);
        let names: Vec<_> = book.underlyings().iter().map(ToString::to_string).collect();
        assert_eq!(names, ["AAPL", "MSFT", "NVDA", "SPY", "TSLA"]);
    }

    #[test]
    fn sample_file_has_each_required_structure() {
        let specs = load_csv(&sample_path()).unwrap();
        let positions = resolve(&specs, &market(start()), start()).unwrap();
        let find = |id: &str| {
            positions
                .iter()
                .find(|p| p.id.as_str() == id)
                .unwrap_or_else(|| unreachable!("missing {id}"))
        };
        let option = |id: &str| match &find(id).instrument {
            Instrument::Option(c) => c.clone(),
            Instrument::Equity { .. } => unreachable!("{id} is not an option"),
        };

        // Long SPY stock and a long put 5 % below the spot, rounded to $5.
        assert!(find("spy-stock").quantity > 0.0);
        let hedge = option("spy-put-hedge");
        assert!((hedge.strike - 570.0).abs() < 1e-9);
        assert!(find("spy-put-hedge").quantity > 0.0);

        // Covered call: long stock, short call above the spot.
        assert!(find("aapl-stock").quantity > 0.0);
        assert!(find("aapl-cc-call").quantity < 0.0);
        assert!(option("aapl-cc-call").strike > 230.0);

        // Long MSFT straddle: same strike and expiry, both long.
        let (call, put) = (option("msft-straddle-call"), option("msft-straddle-put"));
        assert!((call.strike - put.strike).abs() < 1e-9 && call.expiry == put.expiry);
        assert!(
            find("msft-straddle-call").quantity > 0.0 && find("msft-straddle-put").quantity > 0.0
        );

        // Short NVDA straddle at 14 days: first Friday on or after 15 October is 16 October.
        let (call, put) = (option("nvda-straddle-call"), option("nvda-straddle-put"));
        assert!((call.strike - put.strike).abs() < 1e-9 && call.expiry == put.expiry);
        assert_eq!(call.expiry, NaiveDate::from_ymd_opt(2026, 10, 16).unwrap());
        assert!(
            find("nvda-straddle-call").quantity < 0.0 && find("nvda-straddle-put").quantity < 0.0
        );

        // TSLA bull call spread: long the lower strike, short the higher.
        let (long, short) = (option("tsla-spread-long"), option("tsla-spread-short"));
        assert_eq!(long.option_type, short.option_type);
        assert!(long.strike < short.strike && long.expiry == short.expiry);
        assert!(
            find("tsla-spread-long").quantity > 0.0 && find("tsla-spread-short").quantity < 0.0
        );

        // A long ATM SPY call expiring today.
        assert_eq!(
            option("spy-call-expiring").expiry,
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );
        assert!(find("spy-call-expiring").quantity > 0.0);

        // One volatility override.
        let overrides: Vec<_> = positions
            .iter()
            .filter(|p| p.iv_override.is_some())
            .collect();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].id.as_str(), "tsla-put-iv");
    }

    #[test]
    fn day_one_pnl_is_zero_and_the_expiring_call_settles() {
        let specs = load_csv(&sample_path()).unwrap();
        let m = market(start());
        let positions = resolve(&specs, &m, start()).unwrap();
        let mut book = PositionBook::from_positions(positions).unwrap();

        // Blank entry prices give zero unrealised P&L at load.
        let rows = book.value_all(&m, start());
        assert_eq!(rows.len(), 12);
        for row in &rows {
            assert_eq!(row.status, PositionStatus::Active, "{}", row.id);
            assert_abs_diff_eq!(row.unrealised_pnl, 0.0, epsilon = 1e-6);
            assert_abs_diff_eq!(row.realised_pnl, 0.0);
        }

        // After the 16:00 close (20:00 UTC in October) the expiring SPY call settles.
        let after_close = Utc.with_ymd_and_hms(2026, 10, 1, 20, 0, 0).unwrap();
        let events = book.settle_expired(after_close, &m);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].position_id.as_str(), "spy-call-expiring");
        assert_eq!(events[0].kind, InstrumentKind::Call);
        let rows = book.value_all(&m, after_close);
        let expired: Vec<_> = rows
            .iter()
            .filter(|r| r.status == PositionStatus::Expired)
            .collect();
        assert_eq!(expired.len(), 1);
        assert!(
            rows.iter()
                .all(|r| !matches!(r.status, PositionStatus::Error { .. }))
        );
    }
}
