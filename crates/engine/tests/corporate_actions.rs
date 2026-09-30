//! Corporate actions through the engine: a split leaves value and risk unchanged and shows
//! no false P&L, a dividend moves cash into its own bucket, and a restart or a version load
//! never applies a split twice.

// Plain helper functions that tests call into are not covered by clippy's
// `allow-unwrap-in-tests`, and failing fast on a broken fixture is what a test wants.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use approx::assert_relative_eq;
use chrono::NaiveDate;
use domain::{AppliedAction, CashDividend, CorporateAction, FixedClock, SimClock, Symbol};
use engine::{
    CorporateActionCalendar, CorporateActionOutcome, CorporateActionState, CorporateActionsConfig,
    Engine, EngineCommand, EngineReply, GbmSimulator, InstrumentKind, LEDGER_FILE, Position,
    PositionId, PositionSpec, RiskSnapshot, SeedData, VarMethod, apply_ledger_to_seed, to_spec,
};
use tokio_util::sync::CancellationToken;

mod support;

const TICK: Duration = Duration::from_secs(1);

fn sym(s: &str) -> Symbol {
    Symbol::new(s).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The first session after the fixture's start (Monday 28 September) is Tuesday the 29th.
fn tuesday() -> NaiveDate {
    date(2026, 9, 29)
}

fn stock_spec(id: &str, symbol: &str, quantity: f64) -> PositionSpec {
    PositionSpec {
        id: PositionId::new(id).unwrap(),
        underlying: sym(symbol),
        kind: InstrumentKind::Equity,
        strike: None,
        expiry: None,
        quantity,
        entry_price: None,
        iv: None,
        multiplier: None,
        adjusted: None,
        applied_actions: None,
    }
}

/// The fixture book plus 100 AAPL shares.
fn specs() -> Vec<PositionSpec> {
    let mut specs = support::positions();
    specs.push(stock_spec("aapl-stock", "AAPL", 100.0));
    specs
}

fn split_toml(ratio: f64) -> String {
    format!(
        "[[split]]\nid = \"aapl-split\"\nsymbol = \"AAPL\"\nratio = {ratio}\neffective = \"2026-09-29\"\n"
    )
}

fn dividend_toml(amount: f64) -> String {
    format!(
        "[[dividend]]\nid = \"spy-div\"\nsymbol = \"SPY\"\namount = {amount}\nex_date = \"2026-09-29\"\n"
    )
}

fn calendar(toml: &str) -> CorporateActionCalendar {
    let config = CorporateActionsConfig::from_toml_str(toml).unwrap();
    CorporateActionCalendar::resolve(&config, date(2026, 9, 28)).unwrap()
}

/// The fixture seed with SPY on the discrete dividend model and `dividend` going ex on
/// Tuesday, when there is one.
fn seed_with_dividends(dividend: Option<f64>) -> SeedData {
    let mut seed = support::seed_data();
    if let Some(amount) = dividend {
        seed.dividend_schedule.insert(
            sym("SPY"),
            vec![CashDividend {
                ex_date: tuesday(),
                amount,
            }],
        );
    }
    seed
}

fn build(
    seed: &SeedData,
    specs: &[PositionSpec],
    actions: CorporateActionState,
) -> Engine<GbmSimulator> {
    let config = support::sim_config();
    let feed = GbmSimulator::new(seed, &config).unwrap();
    let clock = SimClock::new(support::start(), config.speed).unwrap();
    Engine::new_with_actions(
        clock,
        seed,
        feed,
        specs,
        support::limits_config(),
        Box::new(FixedClock::new(support::start())),
        100,
        support::RNG_SEED,
        VarMethod::DeltaGammaMc,
        actions,
    )
    .unwrap()
}

fn engine_with(toml: &str, seed: &SeedData) -> Engine<GbmSimulator> {
    build(
        seed,
        &specs(),
        CorporateActionState {
            calendar: calendar(toml),
            ledger: Vec::new(),
        },
    )
}

/// Ticks through the rest of Monday's session and the overnight jump: eight one-hour
/// ticks. The last snapshot is the tick that opens Tuesday, where actions fire.
fn tick_to_tuesday_open(engine: &mut Engine<GbmSimulator>) -> Arc<RiskSnapshot> {
    let mut last = None;
    for _ in 0..8 {
        last = Some(engine.tick(TICK).unwrap());
    }
    let snapshot = last.unwrap();
    assert_eq!(snapshot.session_date, tuesday());
    assert_eq!(snapshot.step_kind, engine::StepKind::Overnight);
    snapshot
}

fn position<'a>(snapshot: &'a RiskSnapshot, id: &str) -> &'a engine::PositionRisk {
    snapshot
        .positions
        .iter()
        .find(|p| p.id.as_str() == id)
        .unwrap()
}

fn rel(a: f64, b: f64) {
    assert_relative_eq!(a, b, max_relative = 1e-9, epsilon = 1e-7);
}

#[test]
fn a_split_tick_keeps_value_and_risk_and_shows_no_false_pnl() {
    let seed = seed_with_dividends(None);
    for ratio in [4.0, 1.5, 0.1] {
        let mut control = engine_with("", &seed);
        let mut split = engine_with(&split_toml(ratio), &seed);
        let c = tick_to_tuesday_open(&mut control);
        let s = tick_to_tuesday_open(&mut split);

        // The market moved by the ratio and nothing else.
        rel(
            s.market[&sym("AAPL")].spot,
            c.market[&sym("AAPL")].spot / ratio,
        );
        rel(s.market[&sym("SPY")].spot, c.market[&sym("SPY")].spot);
        // Change since open is measured from the post-split open, so no false −75 %.
        assert!(
            s.market[&sym("AAPL")].change_since_open.abs() < 1e-12,
            "ratio {ratio}: {}",
            s.market[&sym("AAPL")].change_since_open
        );

        // Portfolio value and risk are unchanged.
        let (st, ct) = (&s.summary.totals, &c.summary.totals);
        rel(st.market_value, ct.market_value);
        rel(st.dollar_delta, ct.dollar_delta);
        rel(st.dollar_gamma_1pct, ct.dollar_gamma_1pct);
        rel(st.vega, ct.vega);
        rel(st.theta, ct.theta);

        // The split tick attributes exactly what the same tick attributes without a split:
        // zero for the stock, and only the ordinary overnight decay for the option.
        for id in ["aapl-stock", "aapl-call-30d", "spy-stock"] {
            let (sb, cb) = (
                s.pnl.by_position[&PositionId::new(id).unwrap()].today,
                c.pnl.by_position[&PositionId::new(id).unwrap()].today,
            );
            for (a, b) in [
                (sb.delta, cb.delta),
                (sb.gamma, cb.gamma),
                (sb.vega, cb.vega),
                (sb.theta, cb.theta),
                (sb.residual, cb.residual),
                (sb.actual, cb.actual),
                (sb.explained(), cb.explained()),
            ] {
                assert_relative_eq!(a, b, max_relative = 1e-6, epsilon = 1e-6);
            }
        }
        let stock = s.pnl.by_position[&PositionId::new("aapl-stock").unwrap()].today;
        assert_relative_eq!(stock.actual, 0.0, epsilon = 1e-9);
        assert_relative_eq!(stock.explained(), 0.0, epsilon = 1e-9);

        // The event, the adjusted positions and the badge.
        assert_eq!(s.corporate_actions.len(), 1);
        let event = &s.corporate_actions[0];
        assert_eq!(event.outcome, CorporateActionOutcome::Applied);
        assert_eq!(
            event.positions_adjusted, 2,
            "the AAPL stock and the AAPL call"
        );
        assert!(position(&s, "aapl-call-30d").adjusted);
        assert!(!position(&s, "aapl-stock").adjusted);
        assert!(!position(&s, "spy-stock").adjusted);
        assert!(c.corporate_actions.is_empty());
    }
}

#[test]
fn a_cash_dividend_drops_the_price_and_pays_the_holder_into_its_own_bucket() {
    let seed = seed_with_dividends(Some(2.0));
    let mut control = engine_with("", &seed);
    let mut paying = engine_with(&dividend_toml(2.0), &seed);
    let c = tick_to_tuesday_open(&mut control);
    let s = tick_to_tuesday_open(&mut paying);

    // There is no diffusion overnight, so the price is exactly the dividend lower.
    assert_relative_eq!(
        s.market[&sym("SPY")].spot,
        c.market[&sym("SPY")].spot - 2.0,
        max_relative = 1e-12
    );
    // The open is the ex-dividend price.
    assert!(s.market[&sym("SPY")].change_since_open.abs() < 1e-12);

    let id = PositionId::new("spy-stock").unwrap();
    let today = s.pnl.by_position[&id].today;
    assert_relative_eq!(today.delta, -20.0, epsilon = 1e-9);
    assert_relative_eq!(today.dividends, 20.0, epsilon = 1e-9);
    assert_relative_eq!(today.actual, 0.0, epsilon = 1e-9);
    assert_relative_eq!(today.residual, 0.0, epsilon = 1e-9);
    assert_relative_eq!(s.pnl.portfolio.today.dividends, 20.0, epsilon = 1e-9);
    // Cash is booked to realised P&L, and the holder's total is unchanged.
    let (with_cash, without) = (position(&s, "spy-stock"), position(&c, "spy-stock"));
    assert_relative_eq!(with_cash.realised_pnl, 20.0, epsilon = 1e-9);
    assert_relative_eq!(
        with_cash.realised_pnl + with_cash.unrealised_pnl,
        without.realised_pnl + without.unrealised_pnl,
        epsilon = 1e-9
    );

    let event = &s.corporate_actions[0];
    assert_eq!(event.outcome, CorporateActionOutcome::Applied);
    assert_relative_eq!(event.cash_paid, 20.0, epsilon = 1e-9);
    assert_eq!(event.positions_adjusted, 0);
}

#[test]
fn a_failing_action_is_reported_and_changes_nothing() {
    // No SPY dividend is on the market's schedule, so the calendar's dividend cannot be paid.
    let seed = seed_with_dividends(None);
    let mut engine = engine_with(&dividend_toml(2.0), &seed);
    let mut control = engine_with("", &seed);
    let s = tick_to_tuesday_open(&mut engine);
    let c = tick_to_tuesday_open(&mut control);
    assert_eq!(s.corporate_actions.len(), 1);
    assert_eq!(
        s.corporate_actions[0].outcome,
        CorporateActionOutcome::Failed
    );
    assert_relative_eq!(s.market[&sym("SPY")].spot, c.market[&sym("SPY")].spot);
    assert_relative_eq!(s.pnl.portfolio.today.dividends, 0.0);
}

async fn let_ticks_happen(count: u32) {
    for _ in 0..count {
        tokio::time::advance(TICK).await;
        tokio::task::yield_now().await;
    }
}

async fn specs_of(handle: &engine::EngineHandle) -> Vec<PositionSpec> {
    let EngineReply::Specs(specs) = handle.send(EngineCommand::GetSpecs).await.unwrap().unwrap()
    else {
        unreachable!("expected Specs")
    };
    specs
}

async fn load(handle: &engine::EngineHandle, specs: Vec<PositionSpec>) {
    let reply = handle
        .send(EngineCommand::LoadPortfolio {
            specs,
            source: "test".to_owned(),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reply, EngineReply::Loaded(_)));
}

fn spec_of<'a>(specs: &'a [PositionSpec], id: &str) -> &'a PositionSpec {
    specs.iter().find(|s| s.id.as_str() == id).unwrap()
}

#[tokio::test(start_paused = true)]
async fn the_split_is_recorded_in_the_ledger_events_and_current_json() {
    let seed = seed_with_dividends(None);
    let engine = engine_with(&split_toml(4.0), &seed);
    let (dir, storage) = support::temp_storage();
    let shutdown = CancellationToken::new();
    let (handle, join) = engine.spawn(TICK, storage.clone(), shutdown.clone());
    let_ticks_happen(9).await;
    assert_eq!(handle.latest().corporate_actions.len(), 1);
    shutdown.cancel();
    join.await.unwrap();

    let ledger: Vec<AppliedAction> =
        engine::read_jsonl(&storage.join(&["portfolios", LEDGER_FILE]).unwrap()).unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].id, "aapl-split");
    assert_relative_eq!(ledger[0].amount, 4.0);
    assert!(matches!(ledger[0].action, CorporateAction::Split { .. }));

    let current: Vec<Position> =
        engine::read_json(&storage.join(&["portfolios", "current.json"]).unwrap()).unwrap();
    let stock = current
        .iter()
        .find(|p| p.id.as_str() == "aapl-stock")
        .unwrap();
    assert_relative_eq!(stock.quantity, 400.0);
    assert_eq!(stock.applied_actions, ["aapl-split"]);
    let call = current
        .iter()
        .find(|p| p.id.as_str() == "aapl-call-30d")
        .unwrap();
    assert!(call.adjusted);
    assert_eq!(call.applied_actions, ["aapl-split"]);

    let events: Vec<engine::SessionEvent> = {
        let sessions = storage.join(&["sessions"]).unwrap();
        let session = std::fs::read_dir(sessions)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        engine::read_jsonl(&session.path().join("events.jsonl")).unwrap()
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, engine::SessionEvent::CorporateAction(a) if a.id == "aapl-split")),
        "the corporate action event must be in events.jsonl"
    );
    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn a_restart_restores_the_adjusted_book_and_market_and_never_applies_the_split_twice() {
    let seed = seed_with_dividends(None);
    let engine = engine_with(&split_toml(4.0), &seed);
    let (dir, storage) = support::temp_storage();
    let shutdown = CancellationToken::new();
    let (handle, join) = engine.spawn(TICK, storage.clone(), shutdown.clone());
    let_ticks_happen(9).await;
    shutdown.cancel();
    join.await.unwrap();
    drop(handle);

    // The `last` start: the persisted book, the ledger replayed into the seed, and the
    // configured calendar minus what the ledger says is done.
    let ledger: Vec<AppliedAction> =
        engine::read_jsonl(&storage.join(&["portfolios", LEDGER_FILE]).unwrap()).unwrap();
    let current: Vec<Position> =
        engine::read_json(&storage.join(&["portfolios", "current.json"]).unwrap()).unwrap();
    let restored_specs: Vec<PositionSpec> = current.iter().map(to_spec).collect();
    let mut restored_seed = seed_with_dividends(None);
    apply_ledger_to_seed(&mut restored_seed, &ledger).unwrap();
    let mut pending = calendar(&split_toml(4.0));
    pending.without_applied(&ledger);
    assert!(pending.pending().is_empty());

    let mut restarted = build(
        &restored_seed,
        &restored_specs,
        CorporateActionState {
            calendar: pending,
            ledger,
        },
    );
    let mut last = None;
    for _ in 0..12 {
        last = Some(restarted.tick(TICK).unwrap());
    }
    let snapshot = last.unwrap();
    // Not applied a second time: still 400 shares, the market is still in post-split terms,
    // and no event fired.
    let stock = position(&snapshot, "aapl-stock");
    assert_relative_eq!(stock.quantity, 400.0);
    assert!(
        stock.spot < 100.0,
        "spot {} should be about 57.5",
        stock.spot
    );
    assert!(snapshot.corporate_actions.is_empty());
    // No false P&L from a mismatch between book and market: the stock's day is just
    // diffusion, so it is fully explained.
    let b = snapshot.pnl.by_position[&PositionId::new("aapl-stock").unwrap()].since_start;
    assert_relative_eq!(b.residual, 0.0, epsilon = 1e-6);
    assert!(b.actual.abs() < 0.2 * stock.market_value.abs());
    drop(dir);
}

#[tokio::test(start_paused = true)]
async fn a_version_saved_before_a_split_loads_adjusted_once_and_one_saved_after_loads_unchanged() {
    let seed = seed_with_dividends(None);
    let engine = engine_with(&split_toml(1.5), &seed);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(TICK, storage, CancellationToken::new());

    let before_split = specs_of(&handle).await;
    assert!(
        spec_of(&before_split, "aapl-call-30d")
            .applied_actions
            .is_none()
    );
    let_ticks_happen(9).await;
    let after_split = specs_of(&handle).await;
    let call_after = spec_of(&after_split, "aapl-call-30d");
    assert_eq!(
        call_after.multiplier,
        Some(150.0),
        "3-for-2 changes the multiplier"
    );
    assert_eq!(call_after.adjusted, Some(true));
    assert_eq!(
        call_after.applied_actions.as_deref(),
        Some(["aapl-split".to_owned()].as_slice())
    );

    // The old version, loaded now, is adjusted exactly once and matches the live book.
    load(&handle, before_split).await;
    assert_eq!(specs_of(&handle).await, after_split);
    // Loading the post-split version changes nothing.
    load(&handle, after_split.clone()).await;
    assert_eq!(specs_of(&handle).await, after_split);
    // And loading the result of the first load does not adjust it a second time.
    let reloaded = specs_of(&handle).await;
    load(&handle, reloaded.clone()).await;
    assert_eq!(specs_of(&handle).await, reloaded);
}

#[tokio::test(start_paused = true)]
async fn a_position_added_after_a_split_is_stamped_and_not_adjusted_again_on_reload() {
    let seed = seed_with_dividends(None);
    let engine = engine_with(&split_toml(4.0), &seed);
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(TICK, storage, CancellationToken::new());
    let_ticks_happen(9).await;

    let added = stock_spec("aapl-new", "AAPL", 7.0);
    handle
        .send(EngineCommand::AddPosition(added))
        .await
        .unwrap()
        .unwrap();
    let with_new = specs_of(&handle).await;
    let new = spec_of(&with_new, "aapl-new");
    assert_relative_eq!(new.quantity, 7.0);
    assert_eq!(
        new.applied_actions.as_deref(),
        Some(["aapl-split".to_owned()].as_slice())
    );
    assert_ne!(new.adjusted, Some(true), "nothing was changed, so no badge");

    load(&handle, with_new.clone()).await;
    let reloaded = specs_of(&handle).await;
    assert_relative_eq!(spec_of(&reloaded, "aapl-new").quantity, 7.0);
    assert_eq!(reloaded, with_new);
}
