//! Splits leave a book's value and risk unchanged, and discrete dividends follow the
//! escrowed model. Also the load-time ledger rules for saved books.

// Wrapped in `cfg(test)` so the workspace's allow-unwrap-in-tests setting covers the helpers too.
#[cfg(test)]
mod tests {
    use approx::{assert_abs_diff_eq, assert_relative_eq};
    use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
    use domain::{
        AppliedAction, CashDividend, CorporateAction, Instrument, InstrumentKind, MarketState,
        OptionType, Position, PositionId, SpotAdjustment, Symbol, UnderlyingState, escrow,
        ex_instant, year_fraction,
    };
    use portfolio::{
        PositionBook, PositionRisk, PositionStatus, adjust_specs_for_ledger, ledger_splits,
        parse_csv, resolve, to_csv_bytes, to_spec,
    };

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Thursday 1 October 2026, 10:00 New York.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 14, 0, 0).unwrap()
    }

    const RATE: f64 = 0.04;

    fn dividends() -> Vec<CashDividend> {
        vec![
            CashDividend {
                ex_date: date(2026, 10, 15),
                amount: 1.5,
            },
            CashDividend {
                ex_date: date(2026, 11, 12),
                amount: 1.5,
            },
        ]
    }

    /// AAPL at 230 and SPY at 600. AAPL is on the discrete model when `discrete` is set.
    fn market(discrete: bool) -> MarketState {
        let market = MarketState::new(
            now(),
            RATE,
            [
                (
                    sym("AAPL"),
                    UnderlyingState::new(230.0, 0.25, 0.005).unwrap(),
                ),
                (sym("SPY"), UnderlyingState::new(600.0, 0.16, 0.01).unwrap()),
            ],
        )
        .unwrap();
        if discrete {
            market
                .with_discrete_dividends([(sym("AAPL"), dividends())])
                .unwrap()
        } else {
            market
        }
    }

    fn option(id: &str, kind: OptionType, strike: f64, quantity: f64, entry: f64) -> Position {
        let contract =
            domain::OptionContract::new(sym("AAPL"), kind, strike, date(2026, 12, 18)).unwrap();
        Position::new(
            PositionId::new(id).unwrap(),
            Instrument::Option(contract),
            quantity,
            entry,
            None,
        )
        .unwrap()
    }

    fn stock(id: &str, symbol: &str, quantity: f64, entry: f64) -> Position {
        Position::new(
            PositionId::new(id).unwrap(),
            Instrument::Equity {
                symbol: sym(symbol),
            },
            quantity,
            entry,
            None,
        )
        .unwrap()
    }

    fn book() -> PositionBook {
        let mut with_iv = option("aapl-put", OptionType::Put, 220.0, -2.0, 6.0);
        with_iv.iv_override = Some(0.3);
        PositionBook::from_positions([
            stock("aapl-stock", "AAPL", 100.0, 220.0),
            option("aapl-call", OptionType::Call, 240.0, 3.0, 8.0),
            with_iv,
            stock("spy-stock", "SPY", 10.0, 590.0),
        ])
        .unwrap()
    }

    fn row<'a>(rows: &'a [PositionRisk], id: &str) -> &'a PositionRisk {
        rows.iter().find(|r| r.id.as_str() == id).unwrap()
    }

    fn contract_of(instrument: &Instrument) -> Option<&domain::OptionContract> {
        match instrument {
            Instrument::Option(contract) => Some(contract),
            Instrument::Equity { .. } => None,
        }
    }

    fn rel(a: f64, b: f64) {
        assert_relative_eq!(a, b, max_relative = 1e-9, epsilon = 1e-9);
    }

    /// Applies a split to the book, the market spot and the dividend schedule, as the engine does.
    fn split(book: &mut PositionBook, market: &mut MarketState, ratio: f64, id: &str) {
        book.apply_split(&sym("AAPL"), ratio, id).unwrap();
        market
            .adjust_spot(&sym("AAPL"), SpotAdjustment::DivideBy(ratio))
            .unwrap();
        market.divide_dividends(&sym("AAPL"), ratio);
    }

    #[test]
    fn a_split_leaves_value_and_risk_unchanged_for_every_kind_of_split() {
        for discrete in [false, true] {
            // 4-for-1 and 2-for-1 are whole-number forward splits, 3-for-2 and 1-for-10 are not.
            for ratio in [4.0, 2.0, 1.5, 0.1] {
                let mut market = market(discrete);
                let mut book = book();
                let before = book.value_all(&market, now());
                split(&mut book, &mut market, ratio, "s1");
                let after = book.value_all(&market, now());
                for (b, a) in before.iter().zip(&after) {
                    assert_eq!(a.status, PositionStatus::Active, "{} {ratio}", a.id);
                    rel(a.market_value, b.market_value);
                    rel(a.unrealised_pnl, b.unrealised_pnl);
                    rel(a.reporting.dollar_delta, b.reporting.dollar_delta);
                    rel(a.reporting.dollar_gamma_1pct, b.reporting.dollar_gamma_1pct);
                    rel(a.reporting.vega, b.reporting.vega);
                    rel(a.reporting.theta, b.reporting.theta);
                    rel(a.reporting.rho, b.reporting.rho);
                    if a.underlying.as_str() == "AAPL" {
                        // Share delta counts shares, and a split multiplies the shares.
                        rel(a.reporting.share_delta, b.reporting.share_delta * ratio);
                    } else {
                        rel(a.reporting.share_delta, b.reporting.share_delta);
                    }
                }
            }
        }
    }

    #[test]
    fn options_follow_the_occ_rule_and_only_options_get_the_badge() {
        let mut market = market(false);
        let mut forward = book();
        split(&mut forward, &mut market, 4.0, "fwd");
        let call = forward.get(&PositionId::new("aapl-call").unwrap()).unwrap();
        let contract = contract_of(&call.instrument).unwrap();
        assert_relative_eq!(call.quantity, 12.0);
        assert_relative_eq!(contract.strike, 60.0);
        assert_relative_eq!(contract.multiplier, 100.0);
        assert!(call.adjusted);
        assert_eq!(call.applied_actions, ["fwd"]);
        let shares = forward
            .get(&PositionId::new("aapl-stock").unwrap())
            .unwrap();
        assert_relative_eq!(shares.quantity, 400.0);
        assert_relative_eq!(shares.entry_price, 55.0);
        assert!(!shares.adjusted, "the badge is for contracts only");
        assert_eq!(shares.applied_actions, ["fwd"]);
        // Other underlyings are untouched.
        let spy = forward.get(&PositionId::new("spy-stock").unwrap()).unwrap();
        assert_relative_eq!(spy.quantity, 10.0);
        assert!(spy.applied_actions.is_empty());

        let mut market = self::market(false);
        let mut three_for_two = book();
        split(&mut three_for_two, &mut market, 1.5, "tf");
        let call = three_for_two
            .get(&PositionId::new("aapl-call").unwrap())
            .unwrap();
        let contract = contract_of(&call.instrument).unwrap();
        assert_relative_eq!(call.quantity, 3.0);
        assert_relative_eq!(contract.multiplier, 150.0);
        // OCC would round this strike. This model keeps the exact value.
        assert_relative_eq!(contract.strike, 240.0 / 1.5);
    }

    #[test]
    fn a_reverse_split_on_an_option_keeps_its_value() {
        let mut market = market(false);
        let mut book =
            PositionBook::from_positions([option("aapl-call", OptionType::Call, 240.0, 3.0, 8.0)])
                .unwrap();
        let before = book.value_all(&market, now());
        split(&mut book, &mut market, 0.1, "rev");
        let after = book.value_all(&market, now());
        rel(after[0].market_value, before[0].market_value);
        let contract = contract_of(&book.iter().next().unwrap().instrument).unwrap();
        assert_relative_eq!(contract.strike, 2400.0);
        assert_relative_eq!(contract.multiplier, 10.0);
        assert_relative_eq!(after[0].spot, 2300.0);
    }

    #[test]
    fn applying_the_same_action_twice_changes_nothing_the_second_time() {
        let mut book = book();
        let first = book.apply_split(&sym("AAPL"), 4.0, "s1").unwrap();
        assert_eq!(first.len(), 3);
        let snapshot: Vec<Position> = book.iter().cloned().collect();
        let second = book.apply_split(&sym("AAPL"), 4.0, "s1").unwrap();
        assert!(second.is_empty());
        assert_eq!(book.iter().cloned().collect::<Vec<_>>(), snapshot);
    }

    #[test]
    fn a_settled_option_is_not_adjusted() {
        let mut market = market(false);
        let expired =
            domain::OptionContract::new(sym("AAPL"), OptionType::Call, 200.0, date(2026, 10, 1))
                .unwrap();
        let mut book = PositionBook::from_positions([Position::new(
            PositionId::new("old").unwrap(),
            Instrument::Option(expired),
            1.0,
            5.0,
            None,
        )
        .unwrap()])
        .unwrap();
        let close = Utc.with_ymd_and_hms(2026, 10, 1, 20, 0, 0).unwrap();
        assert_eq!(book.settle_expired(close, &market).len(), 1);
        book.apply_split(&sym("AAPL"), 4.0, "s1").unwrap();
        market
            .adjust_spot(&sym("AAPL"), SpotAdjustment::DivideBy(4.0))
            .unwrap();
        let position = book.iter().next().unwrap();
        assert_relative_eq!(position.quantity, 1.0);
        assert!(!position.adjusted);
    }

    fn pv_of(market: &MarketState, at: DateTime<Utc>, expiry: DateTime<Utc>) -> f64 {
        market
            .dividends(&sym("AAPL"))
            .unwrap()
            .iter()
            .filter(|d| ex_instant(d.ex_date) > at && ex_instant(d.ex_date) <= expiry)
            .map(|d| d.amount * (-RATE * year_fraction(at, ex_instant(d.ex_date))).exp())
            .sum()
    }

    #[test]
    fn put_call_parity_holds_with_discrete_dividends() {
        let market = market(true);
        let strike = 230.0;
        let book = PositionBook::from_positions([
            option("call", OptionType::Call, strike, 1.0, 0.0),
            option("put", OptionType::Put, strike, 1.0, 0.0),
        ])
        .unwrap();
        let rows = book.value_all(&market, now());
        let expiry =
            domain::OptionContract::new(sym("AAPL"), OptionType::Call, strike, date(2026, 12, 18))
                .unwrap()
                .expiry_datetime();
        let t = year_fraction(now(), expiry);
        let parity = 230.0 - pv_of(&market, now(), expiry) - strike * (-RATE * t).exp();
        assert_abs_diff_eq!(
            row(&rows, "call").mark - row(&rows, "put").mark,
            parity,
            epsilon = 1e-10
        );
        // The stored yield is ignored: without the dividends parity would use S·e^(−qT) instead.
        let with_yield = 230.0 * (-0.005 * t).exp() - strike * (-RATE * t).exp();
        assert!((parity - with_yield).abs() > 0.5);
    }

    #[test]
    fn an_options_value_is_continuous_across_the_ex_date_up_to_one_step_of_discounting() {
        let ex = date(2026, 10, 15);
        let d = 1.5;
        // The step is the overnight jump from Wednesday's close to the ex-date's open.
        let step_from = Utc.with_ymd_and_hms(2026, 10, 14, 20, 0, 0).unwrap();
        let before = market(true);
        let book = PositionBook::from_positions([
            option("call", OptionType::Call, 230.0, 1.0, 0.0),
            option("put", OptionType::Put, 230.0, 1.0, 0.0),
        ])
        .unwrap();
        let rows_before = book.value_all(&before, step_from);
        // The ex-date open: the dividend leaves the schedule and spot drops by it.
        let mut after = before.clone();
        assert_eq!(after.take_due_dividends(&sym("AAPL"), ex).len(), 1);
        after
            .adjust_spot(&sym("AAPL"), SpotAdjustment::Subtract(d))
            .unwrap();
        let rows_after = book.value_all(&after, step_from);
        let tau = year_fraction(step_from, ex_instant(ex));
        let gap = d * (1.0 - (-RATE * tau).exp());
        for id in ["call", "put"] {
            let (b, a) = (row(&rows_before, id), row(&rows_after, id));
            // S* moves by exactly the discounting gap, so the value moves by delta × gap.
            assert_abs_diff_eq!(
                a.mark - b.mark,
                -b.greeks.delta * gap,
                epsilon = 1e-6 * gap.max(1e-9)
            );
            assert!(
                (a.mark - b.mark).abs() < 0.01 * d,
                "{id} jumped by {}",
                a.mark - b.mark
            );
        }
    }

    #[test]
    fn a_stock_holders_total_pnl_is_continuous_once_the_dividend_cash_is_counted() {
        let mut market = market(true);
        let mut book =
            PositionBook::from_positions([stock("aapl-stock", "AAPL", 100.0, 220.0)]).unwrap();
        let total = |rows: &[PositionRisk]| rows[0].unrealised_pnl + rows[0].realised_pnl;
        let before = total(&book.value_all(&market, now()));
        // Ex-date: the price drops by the dividend and the holder is paid quantity × amount.
        let amount = 1.5;
        market
            .adjust_spot(&sym("AAPL"), SpotAdjustment::Subtract(amount))
            .unwrap();
        let unrealised_only = total(&book.value_all(&market, now()));
        assert_relative_eq!(unrealised_only, before - 150.0, max_relative = 1e-12);
        let paid = book.pay_dividend(&sym("AAPL"), amount);
        assert_eq!(paid.len(), 1);
        assert_relative_eq!(paid[0].1, 150.0);
        let rows = book.value_all(&market, now());
        assert_relative_eq!(rows[0].realised_pnl, 150.0);
        assert_relative_eq!(total(&rows), before, max_relative = 1e-12);
    }

    #[test]
    fn a_short_stock_position_pays_the_dividend_and_options_receive_nothing() {
        let mut book = PositionBook::from_positions([
            stock("short", "AAPL", -40.0, 220.0),
            option("call", OptionType::Call, 240.0, 3.0, 8.0),
        ])
        .unwrap();
        let paid = book.pay_dividend(&sym("AAPL"), 0.5);
        assert_eq!(paid.len(), 1);
        assert_relative_eq!(paid[0].1, -20.0);
        let rows = book.value_all(&market(true), now());
        assert_relative_eq!(row(&rows, "call").realised_pnl, 0.0);
    }

    /// The per-unit price of one option under `market` at `at`.
    fn price(market: &MarketState, kind: OptionType, at: DateTime<Utc>) -> f64 {
        let book = PositionBook::from_positions([option("o", kind, 230.0, 1.0, 0.0)]).unwrap();
        book.value_all(market, at)[0].mark
    }

    fn bumped(spot: f64, rate: f64) -> MarketState {
        MarketState::new(
            now(),
            rate,
            [
                (
                    sym("AAPL"),
                    UnderlyingState::new(spot, 0.25, 0.005).unwrap(),
                ),
                (sym("SPY"), UnderlyingState::new(600.0, 0.16, 0.01).unwrap()),
            ],
        )
        .unwrap()
        .with_discrete_dividends([(sym("AAPL"), dividends())])
        .unwrap()
    }

    #[test]
    fn finite_differences_confirm_delta_gamma_theta_and_rho_under_the_discrete_model() {
        let base = market(true);
        for kind in [OptionType::Call, OptionType::Put] {
            let book = PositionBook::from_positions([option("o", kind, 230.0, 1.0, 0.0)]).unwrap();
            let g = book.value_all(&base, now())[0].greeks;

            // Delta and gamma: bump the spot with the dividend schedule held fixed.
            let h = 0.05;
            let up = price(&bumped(230.0 + h, RATE), kind, now());
            let down = price(&bumped(230.0 - h, RATE), kind, now());
            let mid = price(&base, kind, now());
            assert_relative_eq!(g.delta, (up - down) / (2.0 * h), max_relative = 1e-6);
            assert_relative_eq!(
                g.gamma,
                (up - 2.0 * mid + down) / (h * h),
                max_relative = 1e-4
            );

            // Theta: value change per year of calendar time passing, with S* moving.
            let dt = Duration::hours(1);
            let dt_years = year_fraction(now(), now() + dt);
            let fd_theta = (price(&base, kind, now() + dt) - price(&base, kind, now() - dt))
                / (2.0 * dt_years);
            assert_relative_eq!(g.theta, fd_theta, max_relative = 1e-5);

            // Rho: bump the rate, which changes both the discounting and PV(D).
            let dr = 1e-5;
            let fd_rho = (price(&bumped(230.0, RATE + dr), kind, now())
                - price(&bumped(230.0, RATE - dr), kind, now()))
                / (2.0 * dr);
            assert_relative_eq!(g.rho, fd_rho, max_relative = 1e-6);

            // The plain BSM Greeks at S* are off: without the corrections theta and rho fail.
            let expiry = domain::OptionContract::new(sym("AAPL"), kind, 230.0, date(2026, 12, 18))
                .unwrap()
                .expiry_datetime();
            let e = escrow(230.0, &dividends(), RATE, now(), expiry).unwrap();
            let inputs = pricing::BsInputs::new(
                e.spot_star,
                230.0,
                year_fraction(now(), expiry),
                RATE,
                0.0,
                0.25,
            )
            .unwrap();
            let plain = pricing::greeks(kind, &inputs);
            assert!((plain.theta - fd_theta).abs() > 10.0 * (g.theta - fd_theta).abs());
            assert!((plain.rho - fd_rho).abs() > 10.0 * (g.rho - fd_rho).abs());
        }
    }

    #[test]
    fn a_non_positive_escrowed_spot_errors_only_that_position() {
        // AAPL trades at 5 with 3 of dividends due, so an option needs S* = 5 − PV.
        // A large dividend leaves nothing for the option. The other rows are unaffected.
        let market = MarketState::new(
            now(),
            RATE,
            [
                (sym("AAPL"), UnderlyingState::new(5.0, 0.25, 0.0).unwrap()),
                (sym("SPY"), UnderlyingState::new(600.0, 0.16, 0.01).unwrap()),
            ],
        )
        .unwrap()
        .with_discrete_dividends([(
            sym("AAPL"),
            vec![CashDividend {
                ex_date: date(2026, 10, 15),
                amount: 10.0,
            }],
        )])
        .unwrap();
        let book = PositionBook::from_positions([
            option("aapl-call", OptionType::Call, 5.0, 1.0, 1.0),
            stock("aapl-stock", "AAPL", 10.0, 5.0),
            stock("spy-stock", "SPY", 1.0, 590.0),
        ])
        .unwrap();
        let rows = book.value_all(&market, now());
        let status = &row(&rows, "aapl-call").status;
        let reason = match status {
            PositionStatus::Error { reason } => reason.as_str(),
            _ => "",
        };
        assert!(reason.contains("escrowed spot"), "{reason}");
        assert_eq!(row(&rows, "aapl-stock").status, PositionStatus::Active);
        assert_eq!(row(&rows, "spy-stock").status, PositionStatus::Active);
    }

    #[test]
    fn the_theoretical_entry_price_uses_the_discrete_model_too() {
        let plain = market(false);
        let discrete = market(true);
        let spec = parse_csv(
            "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
             c,AAPL,call,230,2026-12-18,1,,\n"
                .as_bytes(),
        )
        .unwrap();
        let with_yield = resolve(&spec, &plain, now()).unwrap();
        let escrowed = resolve(&spec, &discrete, now()).unwrap();
        // Dividends of 3 in total lower a call. The two models must disagree.
        assert!(escrowed[0].entry_price < with_yield[0].entry_price - 0.1);
        // And the resolve price equals what valuation marks.
        let book = PositionBook::from_positions(escrowed.clone()).unwrap();
        assert_relative_eq!(
            book.value_all(&discrete, now())[0].mark,
            escrowed[0].entry_price,
            max_relative = 1e-12
        );
    }

    // ---- Load-time ledger rules ----

    fn ledger(id: &str, ratio: f64) -> Vec<AppliedAction> {
        vec![
            AppliedAction {
                id: id.to_owned(),
                action: CorporateAction::split(sym("AAPL"), ratio, date(2026, 10, 2)).unwrap(),
                amount: ratio,
            },
            // Dividends are in the ledger too, and are never re-applied to a book.
            AppliedAction {
                id: "div".to_owned(),
                action: CorporateAction::cash_dividend(sym("AAPL"), 1.0, date(2026, 10, 2))
                    .unwrap(),
                amount: 1.0,
            },
        ]
    }

    fn through_csv(positions: &[Position]) -> Vec<portfolio::PositionSpec> {
        let specs: Vec<_> = positions.iter().map(to_spec).collect();
        parse_csv(to_csv_bytes(&specs).unwrap().as_slice()).unwrap()
    }

    fn reload(
        specs: Vec<portfolio::PositionSpec>,
        ledger: &[AppliedAction],
        market: &MarketState,
    ) -> Vec<Position> {
        resolve(
            &adjust_specs_for_ledger(specs, &ledger_splits(ledger)),
            market,
            now(),
        )
        .unwrap()
    }

    #[test]
    fn a_version_saved_before_a_split_loads_adjusted_exactly_once() {
        let mut market = market(false);
        let pre_split: Vec<Position> = book().iter().cloned().collect();
        let saved = through_csv(&pre_split);

        // The split happens live on a book built from the same positions.
        let mut live = PositionBook::from_positions(pre_split).unwrap();
        split(&mut live, &mut market, 1.5, "tf");
        let expected: Vec<Position> = live.iter().cloned().collect();

        let ledger = ledger("tf", 1.5);
        let loaded = reload(saved.clone(), &ledger, &market);
        assert_eq!(loaded.len(), expected.len());
        for (l, e) in loaded.iter().zip(&expected) {
            assert_eq!(l, e);
        }
        // Loading again from what was just written does not adjust a second time.
        let loaded_twice = reload(through_csv(&loaded), &ledger, &market);
        assert_eq!(loaded_twice, loaded);
    }

    #[test]
    fn a_version_saved_after_a_split_loads_unchanged() {
        let mut market = market(false);
        let mut live = book();
        split(&mut live, &mut market, 4.0, "fwd");
        let post_split: Vec<Position> = live.iter().cloned().collect();
        let loaded = reload(through_csv(&post_split), &ledger("fwd", 4.0), &market);
        assert_eq!(loaded, post_split);
    }

    #[test]
    fn a_position_created_after_a_split_is_stamped_and_never_adjusted_on_reload() {
        let market = market(false);
        // A new position is added in post-split terms: stamped with the ledger's ids, not adjusted.
        let mut added = option("added", OptionType::Call, 60.0, 5.0, 2.0);
        added.applied_actions = vec!["fwd".to_owned()];
        let loaded = reload(through_csv(&[added.clone()]), &ledger("fwd", 4.0), &market);
        assert_eq!(loaded, vec![added]);
        assert!(!loaded[0].adjusted);
    }

    #[test]
    fn specs_priced_at_load_time_are_stamped_but_not_adjusted() {
        let text = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
                    a,AAPL,call,ATM,2026-12-18,1,,\n\
                    b,AAPL,call,ATM,2026-12-18,1,7.5,\n\
                    c,AAPL,equity,,,10,,\n";
        let specs = parse_csv(text.as_bytes()).unwrap();
        let adjusted = adjust_specs_for_ledger(specs.clone(), &ledger_splits(&ledger("fwd", 4.0)));
        for (before, after) in specs.iter().zip(&adjusted) {
            assert_relative_eq!(before.quantity, after.quantity);
            assert_eq!(before.entry_price, after.entry_price);
            assert_eq!(
                after.applied_actions.as_deref(),
                Some(["fwd".to_owned()].as_slice())
            );
            assert_ne!(after.adjusted, Some(true));
        }
    }

    #[test]
    fn splits_in_a_ledger_apply_in_order_and_skip_ones_already_recorded() {
        let mut market = market(false);
        let pre: Vec<Position> = book().iter().cloned().collect();
        let saved_before_both = through_csv(&pre);
        let mut live = PositionBook::from_positions(pre).unwrap();
        split(&mut live, &mut market, 4.0, "first");
        // A version saved between the two splits lists only the first.
        let saved_between = through_csv(&live.iter().cloned().collect::<Vec<_>>());
        split(&mut live, &mut market, 1.5, "second");
        let expected: Vec<Position> = live.iter().cloned().collect();

        let mut both = ledger("first", 4.0);
        both.extend(ledger("second", 1.5).into_iter().take(1));
        assert_eq!(reload(saved_before_both, &both, &market), expected);
        assert_eq!(reload(saved_between, &both, &market), expected);
    }

    #[test]
    fn csv_carries_multiplier_adjusted_and_applied_actions_and_old_files_still_load() {
        let mut market = market(false);
        let mut live = book();
        split(&mut live, &mut market, 1.5, "tf");
        let positions: Vec<Position> = live.iter().cloned().collect();
        let text = String::from_utf8(
            to_csv_bytes(&positions.iter().map(to_spec).collect::<Vec<_>>()).unwrap(),
        )
        .unwrap();
        assert!(
            text.lines()
                .next()
                .unwrap()
                .ends_with("multiplier,adjusted,applied_actions")
        );
        let resolved = resolve(&parse_csv(text.as_bytes()).unwrap(), &market, now()).unwrap();
        assert_eq!(resolved, positions);
        let contract = contract_of(&resolved[1].instrument).unwrap();
        assert_relative_eq!(contract.multiplier, 150.0);

        // A file from before Part 19 has eight columns and loads with the defaults.
        let old = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
                   c,AAPL,call,230,2026-12-18,1,7,\n";
        let specs = parse_csv(old.as_bytes()).unwrap();
        assert_eq!(specs[0].multiplier, None);
        let old_positions = resolve(&specs, &market, now()).unwrap();
        let contract = contract_of(&old_positions[0].instrument).unwrap();
        assert_relative_eq!(contract.multiplier, 100.0);
        assert_eq!(InstrumentKind::Call, specs[0].kind);
    }

    #[test]
    fn a_position_json_from_before_part_19_still_loads() {
        let json = r#"{"id":"p","instrument":{"type":"equity","symbol":"AAPL"},"quantity":1.0,"entry_price":2.0,"iv_override":null}"#;
        let position: Position = serde_json::from_str(json).unwrap();
        assert!(position.applied_actions.is_empty());
        assert!(!position.adjusted);
    }
}
