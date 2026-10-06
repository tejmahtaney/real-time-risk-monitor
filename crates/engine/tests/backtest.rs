//! End-to-end tests of the backtester: the manager, the replay job, the recorder and the files.
//!
//! Every price series here alternates between two values (log returns of +a, -a, +a, ...), so
//! every 20-return window holds ten of each and the rolling volatility is the constant
//! `sqrt(20/19 * a^2 * 252)`. That makes the expected marks and P&L computable by hand.

// Exact float equality is the point of the look-ahead test: a start-of-day VaR that a later price
// cannot touch must be bit-for-bit the same, not merely close. `unwrap_used` is allowed because
// clippy exempts only code inside `#[test]` functions, not the helpers they call.
#![allow(clippy::unwrap_used, clippy::float_cmp)]

mod support;

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, NaiveDate};
use domain::OptionType;
use engine::{
    BacktestConfig, BacktestError, BacktestEvent, BacktestManager, BacktestResults,
    BacktestRunState, BacktestSource, BacktestSplitHandling, ControlCommand,
    CorporateActionOutcome, CreateOptions, MarketConfig, RunInput,
};
use marketdata::{ArchiveRecord, CorporateActionsConfig, RequestParams};
use portfolio::round_strike;
use pricing::{BsInputs, price};
use tokio_util::sync::CancellationToken;

const RATE: f64 = 0.04;
const YIELD: f64 = 0.01;
const A: f64 = 0.01;

fn day(i: usize) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 5).unwrap() + ChronoDuration::days(i64::try_from(i).unwrap())
}

/// Closes that alternate between `start` (even index) and `start * e^a` (odd index).
fn alternating(start: f64, a: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| if i % 2 == 0 { start } else { start * a.exp() })
        .collect()
}

fn long_csv(series: &[(&str, Vec<f64>)]) -> String {
    let mut out = String::from("date,symbol,close\n");
    for (symbol, closes) in series {
        for (i, close) in closes.iter().enumerate() {
            let _ = writeln!(out, "{},{symbol},{close}", day(i));
        }
    }
    out
}

fn wide_csv(series: &[(&str, Vec<f64>)]) -> String {
    let names: Vec<&str> = series.iter().map(|(s, _)| *s).collect();
    let mut out = format!("date,{}\n", names.join(","));
    for i in 0..series[0].1.len() {
        let cells: Vec<String> = series.iter().map(|(_, c)| c[i].to_string()).collect();
        let _ = writeln!(out, "{},{}", day(i), cells.join(","));
    }
    out
}

fn market_config() -> MarketConfig {
    MarketConfig::from_toml_str(&format!(
        r#"
        symbols = ["SPY", "AAPL"]
        risk_free_rate = {RATE}
        vol_window_days = 60
        vol_method = "realised"
        cache_max_age_hours = 24
        data_mode = "offline"
        [dividend_yields]
        SPY = {YIELD}
        AAPL = 0.005
        [dividend_model]
        SPY = "discrete"
        "#
    ))
    .unwrap()
}

const STOCK_AND_CALL: &str = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
    stock,SPY,equity,,,100,,\n\
    call,SPY,call,ATM,+30d,2,,\n";

/// A config whose portfolio is `portfolio_csv`, in a fresh temp storage folder.
fn fixture(portfolio_csv: &str, actions_toml: &str) -> (tempfile::TempDir, BacktestConfig) {
    let dir = tempfile::tempdir().unwrap();
    let portfolio_path = dir.path().join("sample_portfolio.csv");
    std::fs::write(&portfolio_path, portfolio_csv).unwrap();
    let storage = engine::StorageRoot::at(dir.path().join("storage"));
    let config = BacktestConfig {
        storage,
        market: market_config(),
        limits: support::limits_config(),
        headline_var_method: engine::VarMethod::DeltaGammaMc,
        corporate_actions: CorporateActionsConfig::from_toml_str(actions_toml).unwrap(),
        portfolio_path: portfolio_path.clone(),
        config_paths: vec![portfolio_path],
        max_running: 8,
        idle_after: Duration::from_secs(600),
    };
    (dir, config)
}

fn upload(csv: &str) -> RunInput {
    RunInput::Upload {
        filename: Some("prices.csv".to_owned()),
        bytes: csv.as_bytes().to_vec(),
    }
}

async fn wait_for(manager: &BacktestManager, id: &str, wanted: impl Fn(BacktestRunState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let state = manager.status(id).unwrap().state;
        if wanted(state) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out in state {state:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Creates a run, runs it to the end and returns its results.
async fn run_all(manager: &BacktestManager, input: RunInput, roll: bool) -> BacktestResults {
    let id = manager
        .create(
            input,
            CreateOptions {
                roll_expired: roll,
                ..CreateOptions::default()
            },
        )
        .await
        .unwrap()
        .id;
    manager.control(&id, ControlCommand::RunToEnd).unwrap();
    wait_for(manager, &id, |s| s == BacktestRunState::Finished).await;
    manager.results(&id).unwrap()
}

/// The call's mark by direct Black-Scholes-Merton. SPY is on the discrete dividend model in the
/// test market config, so its continuous yield is zero.
fn call_mark(spot: f64, strike: f64, days_to_expiry: i64) -> f64 {
    let vol = (20.0 / 19.0 * A * A * 252.0).sqrt();
    let t = days_to_expiry as f64 / 365.0;
    price(
        OptionType::Call,
        &BsInputs::new(spot, strike, t, RATE, 0.0, vol).unwrap(),
    )
}

// --- 1. Hand-computed results ------------------------------------------------------------

#[tokio::test]
async fn a_stock_and_a_call_over_thirty_dates_match_hand_computed_results() {
    let (_dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    // 20 warm-up dates, the start date (index 20) and 30 replayed dates: 51 in all.
    let closes = alternating(100.0, A, 51);
    let results = run_all(
        &manager,
        upload(&long_csv(&[("SPY", closes.clone())])),
        false,
    )
    .await;

    assert_eq!(results.rows.len(), 30);
    assert_eq!(results.settings.warm_up_dates, 20);
    assert_eq!(results.settings.start_date, day(20));
    assert_eq!(results.rows.first().unwrap().date, day(21));
    assert_eq!(results.rows.last().unwrap().date, day(50));
    assert!(!results.partial);

    // The book is resolved at the start date: spot 100, an ATM strike of 100, and a first Friday
    // on or after start + 30 days: 2026-01-25 + 30 = Tuesday 24 February, so Friday 27 February.
    let strike = round_strike(closes[20]);
    let expiry = NaiveDate::from_ymd_opt(2026, 2, 27).unwrap();
    let mut cumulative = 0.0;
    for (offset, row) in results.rows.iter().enumerate() {
        let i = 21 + offset;
        assert_eq!(row.date, day(i));
        let stock = 100.0 * (closes[i] - closes[i - 1]);
        let days_now = (expiry - day(i)).num_days();
        let option = 2.0
            * 100.0
            * (call_mark(closes[i], strike, days_now)
                - call_mark(closes[i - 1], strike, days_now + 1));
        approx::assert_relative_eq!(
            row.actual,
            stock + option,
            max_relative = 1e-9,
            epsilon = 1e-9
        );
        // The buckets and the residual always add back to the actual change.
        let explained = row.delta
            + row.gamma
            + row.vega
            + row.theta
            + row.rho
            + row.higher_order
            + row.dividends;
        approx::assert_abs_diff_eq!(explained + row.residual, row.actual, epsilon = 1e-9);
        cumulative += row.actual;
        approx::assert_relative_eq!(
            row.cumulative_pnl,
            cumulative,
            max_relative = 1e-9,
            epsilon = 1e-9
        );
        assert!(row.var_99 > 0.0);
    }
    // The stock ends where it started, so the total is the option's decay.
    let option_total =
        200.0 * (call_mark(closes[50], strike, 3) - call_mark(closes[20], strike, 33));
    approx::assert_relative_eq!(
        cumulative,
        option_total,
        max_relative = 1e-9,
        epsilon = 1e-9
    );
    approx::assert_relative_eq!(
        results.summary.total.actual,
        cumulative,
        max_relative = 1e-9,
        epsilon = 1e-9
    );
    assert_eq!(results.summary.days, 30);
    assert_eq!(results.summary.var_backtest.unwrap().n, 30);
    assert!(results.summary.var_backtest.unwrap().low_power);
}

// --- 2. Long and wide forms ---------------------------------------------------------------

#[tokio::test]
async fn long_and_wide_forms_give_the_same_result() {
    let series = [
        ("SPY", alternating(100.0, A, 45)),
        ("AAPL", alternating(230.0, 0.02, 45)),
    ];
    let portfolio = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
        spy,SPY,equity,,,100,,\n\
        aapl,AAPL,call,ATM,+30d,3,,\n";
    let (_dir, config) = fixture(portfolio, "");
    let manager = BacktestManager::new(config);
    let long = run_all(&manager, upload(&long_csv(&series)), true).await;
    let wide = run_all(&manager, upload(&wide_csv(&series)), true).await;
    assert_eq!(long.rows, wide.rows);
    assert_eq!(long.summary, wide.summary);
    // Events carry the real time they were raised, which differs between two runs.
    let simulated = |events: &[BacktestEvent]| -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                BacktestEvent::Alert(a) => {
                    format!("{} {:?} {:?}->{:?}", a.sim_time, a.name, a.from, a.to)
                }
                other => format!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(simulated(&long.events), simulated(&wide.events));
}

// --- 3. Validation errors reach the caller with their line ---------------------------------

#[tokio::test]
async fn bad_uploads_are_rejected_with_the_line_number_and_nothing_is_saved() {
    let (dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    let mut text = long_csv(&[("SPY", alternating(100.0, A, 30))]);
    text += "2026-03-01,SPY,-5\n";
    let error = manager
        .create(upload(&text), CreateOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, BacktestError::Data(_)));
    assert!(error.to_string().starts_with("line 32:"), "{error}");
    let missing = manager
        .create(
            upload(&long_csv(&[("QQQ", alternating(100.0, A, 30))])),
            CreateOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("SPY"), "{missing}");
    assert!(manager.list().unwrap().is_empty());
    assert!(!dir.path().join("storage").join("backtests").exists());
}

// --- 6. The live engine keeps ticking while a backtest runs --------------------------------

/// The shortest backtest that tests anything, and how many times the date count may double to
/// reach it.
const MIN_RUN: Duration = Duration::from_millis(300);
const MAX_DOUBLINGS: u32 = 6;

#[tokio::test(flavor = "current_thread")]
async fn the_live_engine_keeps_ticking_while_a_backtest_runs() {
    let (_dir, storage) = support::temp_storage();
    let shutdown = CancellationToken::new();
    let (handle, engine_task) =
        support::build_engine().spawn(Duration::from_millis(10), storage, shutdown.clone());

    let (_bt_dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);

    // A fast build can replay a fixed number of dates in well under the window this test
    // needs, so it sizes itself: while a run finishes in under MIN_RUN, double the dates and
    // try again, up to MAX_DOUBLINGS. That makes it hold in debug and release by construction.
    let mut dates = 160;
    let mut attempt = 0;
    let (id, before, last_seq, elapsed, longest_gap) = loop {
        let id = manager
            .create(
                upload(&long_csv(&[("SPY", alternating(100.0, A, dates))])),
                CreateOptions::default(),
            )
            .await
            .unwrap()
            .id;
        let before = handle.latest().seq;
        manager.control(&id, ControlCommand::RunToEnd).unwrap();

        let started = Instant::now();
        let mut last_seq = before;
        let mut last_change = Instant::now();
        let mut longest_gap = Duration::ZERO;
        while manager.status(&id).unwrap().state == BacktestRunState::RunningToEnd {
            tokio::time::sleep(Duration::from_millis(2)).await;
            let seq = handle.latest().seq;
            if seq != last_seq {
                longest_gap = longest_gap.max(last_change.elapsed());
                last_seq = seq;
                last_change = Instant::now();
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= MIN_RUN || attempt == MAX_DOUBLINGS {
            break (id, before, last_seq, elapsed, longest_gap);
        }
        dates *= 2;
        attempt += 1;
    };
    assert!(
        elapsed >= MIN_RUN,
        "the backtest was too short to test anything even with {dates} dates: {elapsed:?}"
    );
    assert!(
        last_seq - before >= 10,
        "the live engine only ticked {} times in {elapsed:?}",
        last_seq - before
    );
    assert!(
        longest_gap < Duration::from_millis(300),
        "the live engine stalled for {longest_gap:?} while the backtest ran"
    );
    assert_eq!(
        manager.status(&id).unwrap().state,
        BacktestRunState::Finished
    );

    shutdown.cancel();
    engine_task.await.unwrap();
    manager.shutdown().await;
}

// --- 7. Rolling ----------------------------------------------------------------------------

#[tokio::test]
async fn an_expired_option_is_replaced_by_one_with_the_same_relative_spec() {
    let portfolio = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
        stock,SPY,equity,,,100,,\n\
        opt,SPY,call,ATM,+3d,2,,\n";
    let (_dir, config) = fixture(portfolio, "");
    let manager = BacktestManager::new(config);
    let closes = alternating(100.0, A, 51);
    let results = run_all(
        &manager,
        upload(&long_csv(&[("SPY", closes.clone())])),
        true,
    )
    .await;

    let settlements: Vec<_> = results
        .events
        .iter()
        .filter_map(|e| match e {
            BacktestEvent::Settlement(s) => Some(s),
            _ => None,
        })
        .collect();
    let rolls: Vec<_> = results
        .events
        .iter()
        .filter_map(|e| match e {
            BacktestEvent::Roll(r) => Some(r),
            _ => None,
        })
        .collect();
    // Each option lives until the Friday after start + 3 days, so a 30-date replay sees several.
    assert!(settlements.len() >= 3, "{settlements:?}");
    assert_eq!(settlements.len(), rolls.len(), "every settlement is rolled");
    assert_eq!(settlements[0].position_id.as_str(), "opt");
    assert_eq!(rolls[0].settled.as_str(), "opt");
    assert_eq!(rolls[0].opened.as_str(), "opt-r1");
    assert_eq!(rolls[1].settled.as_str(), "opt-r1");
    assert_eq!(rolls[1].opened.as_str(), "opt-r2");
    // The new option is an ATM call at the roll date's spot, like the row it came from.
    for roll in &rolls {
        let i = (0..closes.len())
            .find(|&i| day(i) == roll.session_date)
            .unwrap();
        let expected = format!("C {}", round_strike(closes[i]));
        assert!(
            roll.description.ends_with(&expected),
            "{} should end with {expected}",
            roll.description
        );
    }
    // The book is still alive at the end of the replay: an active option stands where it was.
    let id = &results.settings.run_id;
    let mut subscription = manager.subscribe(id).unwrap();
    let last = subscription.snapshots.borrow_and_update().clone();
    let active_options = last
        .positions
        .iter()
        .filter(|p| p.status == engine::PositionStatus::Active && p.id.as_str().starts_with("opt"))
        .count();
    assert_eq!(active_options, 1);

    // Without rolling, the same run has settlements and no replacements.
    let plain = run_all(&manager, upload(&long_csv(&[("SPY", closes)])), false).await;
    assert!(
        plain
            .events
            .iter()
            .any(|e| matches!(e, BacktestEvent::Settlement(_)))
    );
    assert!(
        !plain
            .events
            .iter()
            .any(|e| matches!(e, BacktestEvent::Roll(_)))
    );
}

#[tokio::test]
async fn a_zero_day_row_rolls_into_a_one_day_option_every_day() {
    let portfolio = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
        stock,SPY,equity,,,100,,\n\
        zero,SPY,call,ATM,+0d,5,,\n";
    let (_dir, config) = fixture(portfolio, "");
    let manager = BacktestManager::new(config);
    let results = run_all(
        &manager,
        upload(&long_csv(&[("SPY", alternating(100.0, A, 45))])),
        true,
    )
    .await;
    let rolls = results
        .events
        .iter()
        .filter(|e| matches!(e, BacktestEvent::Roll(_)))
        .count();
    // One option opened at each close, settling the next day: one roll per replayed date but the
    // last does not matter to the count check, which only needs it to keep going.
    assert!(
        rolls >= results.rows.len() - 1,
        "{rolls} rolls over {} dates",
        results.rows.len()
    );
}

// --- 8. A finished run reloads from disk ---------------------------------------------------

#[tokio::test]
async fn a_finished_run_reloads_after_a_restart_with_identical_results() {
    let (dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config.clone());
    let csv = long_csv(&[("SPY", alternating(100.0, A, 45))]);
    let results = run_all(&manager, upload(&csv), true).await;
    let id = results.settings.run_id.clone();
    // Stopping waits for the recorder, so every file is complete afterwards.
    manager.stop(&id).await.unwrap();

    let run_dir = dir.path().join("storage").join("backtests").join(&id);
    for file in [
        "input.csv",
        "settings.json",
        "daily.csv",
        "events.jsonl",
        "summary.json",
    ] {
        // events.jsonl exists only when there were events.
        if file != "events.jsonl" || !results.events.is_empty() {
            assert!(run_dir.join(file).exists(), "{file} missing");
        }
    }
    assert_eq!(
        std::fs::read_to_string(run_dir.join("input.csv")).unwrap(),
        csv
    );
    assert!(run_dir.join("config").join("sample_portfolio.csv").exists());

    // A brand-new manager over the same folder: no replay, everything from the files.
    let restarted = BacktestManager::new(config);
    let reloaded = restarted.results(&id).unwrap();
    assert_eq!(reloaded.rows, results.rows);
    assert_eq!(reloaded.summary, results.summary);
    assert_eq!(reloaded.events, results.events);
    assert_eq!(reloaded.settings, results.settings);
    assert!(!reloaded.partial);
    assert!(!reloaded.status.in_memory);
    assert_eq!(reloaded.status.state, BacktestRunState::Finished);

    let listed = restarted.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].state, BacktestRunState::Finished);
    assert_eq!(
        listed[0].source,
        BacktestSource::Upload {
            filename: Some("prices.csv".to_owned())
        }
    );
    approx::assert_relative_eq!(listed[0].total_pnl.unwrap(), results.summary.total.actual);
    // A finished run cannot be controlled: it is not in memory.
    assert!(matches!(
        restarted.control(&id, ControlCommand::Play),
        Err(BacktestError::NotFound(_))
    ));
}

#[tokio::test]
async fn a_run_stopped_part_way_is_listed_as_unfinished_and_keeps_its_rows() {
    let (_dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config.clone());
    let id = manager
        .create(
            upload(&long_csv(&[("SPY", alternating(100.0, A, 45))])),
            CreateOptions::default(),
        )
        .await
        .unwrap()
        .id;
    for _ in 0..3 {
        manager.control(&id, ControlCommand::Step).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while manager.status(&id).unwrap().steps_done < 3 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    manager.stop(&id).await.unwrap();

    let restarted = BacktestManager::new(config);
    let listed = restarted.list().unwrap();
    assert_eq!(listed[0].state, BacktestRunState::Unfinished);
    assert_eq!(listed[0].days, 3);
    let results = restarted.results(&id).unwrap();
    assert!(results.partial);
    assert_eq!(results.rows.len(), 3);
    assert_eq!(results.status.state, BacktestRunState::Unfinished);
}

// --- Look-ahead -----------------------------------------------------------------------------

#[tokio::test]
async fn a_price_on_or_after_a_date_cannot_change_that_dates_start_of_day_var() {
    let (_dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    let base = alternating(100.0, A, 51);
    // The same series except one close: index 35 is 6 % higher.
    let mut bumped = base.clone();
    bumped[35] *= 1.06;
    let a = run_all(&manager, upload(&long_csv(&[("SPY", base)])), false).await;
    let b = run_all(&manager, upload(&long_csv(&[("SPY", bumped)])), false).await;

    let row =
        |r: &BacktestResults, i: usize| r.rows.iter().find(|x| x.date == day(i)).unwrap().clone();
    for i in 21..35 {
        assert_eq!(
            row(&a, i),
            row(&b, i),
            "rows before the change must be identical"
        );
    }
    // On the day of the changed price: the P&L differs but the start-of-day VaR does not.
    assert_ne!(row(&a, 35).actual, row(&b, 35).actual);
    assert_eq!(row(&a, 35).var_99, row(&b, 35).var_99);
    // The next day's start-of-day VaR is computed at the close of the changed date, so it may
    // (and here does) differ.
    assert_ne!(row(&a, 36).var_99, row(&b, 36).var_99);
}

// --- Idle sweep ------------------------------------------------------------------------------

#[tokio::test]
async fn only_a_run_that_is_idle_in_every_way_is_freed() {
    let (_dir, mut config) = fixture(STOCK_AND_CALL, "");
    // Zero idle time: the only things keeping a run are what the sweep is meant to respect.
    config.idle_after = Duration::ZERO;
    let manager = BacktestManager::new(config);
    let csv = long_csv(&[("SPY", alternating(100.0, A, 400))]);
    let create = || async {
        manager
            .create(
                upload(&csv),
                CreateOptions {
                    roll_expired: true,
                    speed: 1.0,
                },
            )
            .await
            .unwrap()
            .id
    };
    let playing_with_client = create().await;
    let playing_alone = create().await;
    let paused_with_client = create().await;
    let paused_alone = create().await;
    let to_end = create().await;

    manager
        .control(&playing_with_client, ControlCommand::Play)
        .unwrap();
    manager
        .control(&playing_alone, ControlCommand::Play)
        .unwrap();
    manager.control(&to_end, ControlCommand::RunToEnd).unwrap();
    let client_a = manager.subscribe(&playing_with_client).unwrap();
    let client_b = manager.subscribe(&paused_with_client).unwrap();

    let freed = manager.sweep(Instant::now() + Duration::from_secs(1)).await;
    assert_eq!(
        freed,
        std::slice::from_ref(&paused_alone),
        "only the paused run with no client goes"
    );
    assert!(!manager.status(&paused_alone).unwrap().in_memory);
    for id in [
        &playing_with_client,
        &playing_alone,
        &paused_with_client,
        &to_end,
    ] {
        assert!(manager.status(id).unwrap().in_memory, "{id} must survive");
    }
    assert_eq!(
        manager.status(&playing_with_client).unwrap().state,
        BacktestRunState::Playing
    );

    // Once the paused run's client disconnects it is idle too; the playing runs still are not.
    drop(client_b);
    let freed = manager.sweep(Instant::now() + Duration::from_secs(1)).await;
    assert_eq!(freed, [paused_with_client]);
    // Pausing the run with a client does not free it while the client stays.
    manager
        .control(&playing_with_client, ControlCommand::Pause)
        .unwrap();
    assert!(
        manager
            .sweep(Instant::now() + Duration::from_secs(1))
            .await
            .is_empty()
    );
    drop(client_a);
    let freed = manager.sweep(Instant::now() + Duration::from_secs(1)).await;
    assert_eq!(freed, [playing_with_client]);
    manager.shutdown().await;
}

#[tokio::test]
async fn a_run_used_recently_is_not_freed_before_the_idle_time() {
    let (_dir, mut config) = fixture(STOCK_AND_CALL, "");
    config.idle_after = Duration::from_secs(600);
    let manager = BacktestManager::new(config);
    let id = manager
        .create(
            upload(&long_csv(&[("SPY", alternating(100.0, A, 45))])),
            CreateOptions::default(),
        )
        .await
        .unwrap()
        .id;
    assert!(
        manager
            .sweep(Instant::now() + Duration::from_secs(599))
            .await
            .is_empty()
    );
    manager.status(&id).unwrap(); // any API call restarts the clock
    assert!(
        manager
            .sweep(Instant::now() + Duration::from_secs(599))
            .await
            .is_empty()
    );
    assert_eq!(
        manager
            .sweep(Instant::now() + Duration::from_secs(601))
            .await,
        [id]
    );
}

// --- Limits and control -------------------------------------------------------------------------

#[tokio::test]
async fn the_running_limit_and_the_speed_range_are_enforced() {
    let (_dir, mut config) = fixture(STOCK_AND_CALL, "");
    config.max_running = 2;
    let manager = BacktestManager::new(config);
    let csv = long_csv(&[("SPY", alternating(100.0, A, 45))]);
    let first = manager
        .create(upload(&csv), CreateOptions::default())
        .await
        .unwrap()
        .id;
    let second = manager
        .create(upload(&csv), CreateOptions::default())
        .await
        .unwrap()
        .id;
    let third = manager.create(upload(&csv), CreateOptions::default()).await;
    assert!(
        matches!(third, Err(BacktestError::TooManyRunning { limit: 2, .. })),
        "{third:?}"
    );

    // A finished run no longer counts.
    manager.control(&first, ControlCommand::RunToEnd).unwrap();
    wait_for(&manager, &first, |s| s == BacktestRunState::Finished).await;
    manager
        .create(upload(&csv), CreateOptions::default())
        .await
        .unwrap();

    for bad in [0.5, 51.0, f64::NAN] {
        assert!(matches!(
            manager.control(&second, ControlCommand::SetSpeed(bad)),
            Err(BacktestError::Invalid(_))
        ));
    }
    assert_eq!(
        manager
            .control(&second, ControlCommand::SetSpeed(50.0))
            .unwrap()
            .speed,
        50.0
    );
    assert!(matches!(
        manager.control(&first, ControlCommand::Play),
        Err(BacktestError::Invalid(_))
    ));
    manager.shutdown().await;
}

// --- 10. Delete -------------------------------------------------------------------------------------

#[tokio::test]
async fn deleting_needs_the_matching_confirmation_and_touches_only_that_folder() {
    let (dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    let csv = long_csv(&[("SPY", alternating(100.0, A, 45))]);
    let keep = run_all(&manager, upload(&csv), false).await.settings.run_id;
    let target = run_all(&manager, upload(&csv), false).await.settings.run_id;
    let base = dir.path().join("storage").join("backtests");

    for wrong in ["", "yes", &keep, "..", "../backtests"] {
        let error = manager.delete_files(&target, wrong).await.unwrap_err();
        assert!(
            matches!(error, BacktestError::Invalid(_)),
            "{wrong:?}: {error:?}"
        );
        assert!(base.join(&target).join("summary.json").exists());
        assert!(base.join(&keep).join("summary.json").exists());
    }
    manager.delete_files(&target, &target).await.unwrap();
    assert!(!base.join(&target).exists());
    assert!(
        base.join(&keep).join("summary.json").exists(),
        "the other run stays"
    );
    assert!(matches!(
        manager.delete_files(&target, &target).await,
        Err(BacktestError::NotFound(_))
    ));
    assert!(matches!(
        manager.results(&target),
        Err(BacktestError::NotFound(_))
    ));
}

#[tokio::test]
async fn deleting_a_run_in_memory_stops_it_first() {
    let (dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    let id = manager
        .create(
            upload(&long_csv(&[("SPY", alternating(100.0, A, 400))])),
            CreateOptions::default(),
        )
        .await
        .unwrap()
        .id;
    manager.control(&id, ControlCommand::RunToEnd).unwrap();
    manager.delete_files(&id, &id).await.unwrap();
    assert!(
        !dir.path()
            .join("storage")
            .join("backtests")
            .join(&id)
            .exists()
    );
    assert!(matches!(
        manager.status(&id),
        Err(BacktestError::NotFound(_))
    ));
}

// --- The archive ---------------------------------------------------------------------------------------

fn alpha_body(closes: &[(NaiveDate, f64)]) -> String {
    let entries: Vec<String> = closes
        .iter()
        .map(|(date, close)| {
            format!(
                r#""{date}":{{"1. open":"{close}","2. high":"{close}","3. low":"{close}","4. close":"{close}","5. volume":"1"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"Meta Data":{{"1. Information":"Daily Prices"}},"Time Series (Daily)":{{{}}}}}"#,
        entries.join(",")
    )
}

fn save_download(storage: &Path, symbol: &str, stamp: &str, closes: &[(NaiveDate, f64)]) {
    let folder = storage.join("market-data").join(symbol);
    std::fs::create_dir_all(&folder).unwrap();
    let record = ArchiveRecord {
        symbol: domain::Symbol::new(symbol).unwrap(),
        fetched_at: chrono::DateTime::parse_from_rfc3339("2026-01-05T00:00:00Z")
            .unwrap()
            .into(),
        params: RequestParams {
            function: "TIME_SERIES_DAILY".into(),
            symbol: symbol.into(),
            outputsize: "compact".into(),
            datatype: "json".into(),
        },
        body: alpha_body(closes),
    };
    engine::write_json_atomic(&folder.join(format!("{stamp}-daily.json")), &record).unwrap();
}

#[tokio::test]
async fn the_archive_source_merges_overlapping_downloads_and_saves_them_as_input() {
    let (dir, config) = fixture(STOCK_AND_CALL, "");
    let storage = dir.path().join("storage");
    let closes = alternating(100.0, A, 60);
    let dated: Vec<(NaiveDate, f64)> = closes
        .iter()
        .enumerate()
        .map(|(i, c)| (day(i), *c))
        .collect();
    // Two downloads that overlap on indices 20 to 39.
    save_download(&storage, "SPY", "20260301T000000Z", &dated[..40]);
    save_download(&storage, "SPY", "20260302T000000Z", &dated[20..]);
    let manager = BacktestManager::new(config);
    let results = run_all(&manager, RunInput::Archive, false).await;

    assert_eq!(results.settings.source, BacktestSource::Archive);
    assert_eq!(results.settings.first_date, day(0));
    assert_eq!(results.settings.last_date, day(59));
    assert_eq!(results.rows.len(), 39);
    let input = std::fs::read_to_string(
        storage
            .join("backtests")
            .join(&results.settings.run_id)
            .join("input.csv"),
    )
    .unwrap();
    // The merged prices, one line per date with no duplicates, plus the header.
    assert_eq!(input.lines().count(), 61);
    assert!(input.starts_with("date,symbol,close\n2026-01-05,SPY,100\n"));

    // A portfolio symbol with no archived history is an error, not a partial run.
    let (_dir2, config2) = fixture(
        "id,underlying,kind,strike,expiry,quantity,entry_price,iv\nx,TSLA,equity,,,1,,\n",
        "",
    );
    let empty = BacktestManager::new(config2);
    let error = empty
        .create(RunInput::Archive, CreateOptions::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("TSLA"), "{error}");
}

// --- Corporate actions --------------------------------------------------------------------------------

/// SPY alternating with a real 4-for-1 split at index 30: from there on prices are a quarter.
fn split_prices() -> Vec<f64> {
    alternating(100.0, A, 51)
        .into_iter()
        .enumerate()
        .map(|(i, c)| if i >= 30 { c / 4.0 } else { c })
        .collect()
}

const SPLIT_CONFIG: &str = r#"
    [[split]]
    id = "spy-4-for-1"
    symbol = "SPY"
    ratio = 4.0
    effective = "2026-02-04"
"#;

#[tokio::test]
async fn a_real_split_never_shows_as_a_collapse_whichever_way_it_is_handled() {
    let csv = long_csv(&[("SPY", split_prices())]);

    // Not configured: the history is restated, so the book never notices.
    let (_dir, config) = fixture(STOCK_AND_CALL, "");
    let manager = BacktestManager::new(config);
    let restated = run_all(&manager, upload(&csv), false).await;
    assert_eq!(restated.settings.splits.len(), 1);
    assert_eq!(
        restated.settings.splits[0].handling,
        BacktestSplitHandling::BackAdjusted
    );
    assert!(
        !restated
            .events
            .iter()
            .any(|e| matches!(e, BacktestEvent::CorporateAction(_)))
    );

    // Configured on its date: prices stay as traded and the engine adjusts the book.
    let (_dir, config) = fixture(STOCK_AND_CALL, SPLIT_CONFIG);
    let manager = BacktestManager::new(config);
    let applied = run_all(&manager, upload(&csv), false).await;
    assert_eq!(
        applied.settings.splits[0].handling,
        BacktestSplitHandling::AppliedByEngine
    );
    let fired: Vec<_> = applied
        .events
        .iter()
        .filter_map(|e| match e {
            BacktestEvent::CorporateAction(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].outcome, CorporateActionOutcome::Applied);
    assert_eq!(fired[0].sim_time.date_naive(), day(30));

    // The book was worth about 10,000 in stock alone. A -75 % shock would be about -7,500; the
    // real move is about 1 %, so both handlings keep the split day small.
    for results in [&restated, &applied] {
        let split_day = results.rows.iter().find(|r| r.date == day(30)).unwrap();
        assert!(split_day.actual.abs() < 1_500.0, "{}", split_day.actual);
        assert!(results.rows.iter().all(|r| r.actual.abs() < 1_500.0));
    }
}

#[tokio::test]
async fn a_configured_split_with_no_price_break_is_skipped_and_demo_events_are_ignored() {
    let actions = r#"
        [[split]]
        id = "no-break-here"
        symbol = "SPY"
        ratio = 2.0
        effective = "2026-02-04"
        [[split]]
        id = "demo"
        symbol = "SPY"
        ratio = 3.0
        effective = "+2d"
        [[dividend]]
        id = "demo-dividend"
        symbol = "SPY"
        amount = 1.0
        ex_date = "+3d"
    "#;
    let (_dir, config) = fixture(STOCK_AND_CALL, actions);
    let manager = BacktestManager::new(config);
    let results = run_all(
        &manager,
        upload(&long_csv(&[("SPY", alternating(100.0, A, 51))])),
        false,
    )
    .await;
    assert!(
        !results
            .events
            .iter()
            .any(|e| matches!(e, BacktestEvent::CorporateAction(_)))
    );
    assert_eq!(results.settings.relative_actions_skipped, 2);
    assert!(results.settings.splits.is_empty());
}

#[tokio::test]
async fn a_dividend_pays_cash_without_lowering_the_price_a_second_time() {
    let portfolio =
        "id,underlying,kind,strike,expiry,quantity,entry_price,iv\nstock,SPY,equity,,,100,,\n";
    let actions = r#"
        [[dividend]]
        id = "spy-div"
        symbol = "SPY"
        amount = 0.5
        ex_date = "2026-02-02"
    "#;
    let (_dir, config) = fixture(portfolio, actions);
    let manager = BacktestManager::new(config);
    let closes = alternating(100.0, A, 51);
    let id = manager
        .create(
            upload(&long_csv(&[("SPY", closes.clone())])),
            CreateOptions::default(),
        )
        .await
        .unwrap()
        .id;
    let mut subscription = manager.subscribe(&id).unwrap();
    // Step to the ex-date, index 28.
    for _ in 21..=28 {
        manager.control(&id, ControlCommand::Step).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while manager.status(&id).unwrap().steps_done < 8 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let snapshot = subscription.snapshots.borrow_and_update().clone();
    let spot = snapshot.market.values().next().unwrap().spot;
    // The real close already fell on the ex-date; the engine must not lower it again.
    approx::assert_relative_eq!(spot, closes[28], max_relative = 1e-12);

    manager.control(&id, ControlCommand::RunToEnd).unwrap();
    wait_for(&manager, &id, |s| s == BacktestRunState::Finished).await;
    let results = manager.results(&id).unwrap();
    let ex = results.rows.iter().find(|r| r.date == day(28)).unwrap();
    approx::assert_relative_eq!(ex.dividends, 50.0);
    // The day's P&L is the price change plus the cash, nothing more.
    approx::assert_relative_eq!(
        ex.actual,
        100.0 * (closes[28] - closes[27]) + 50.0,
        max_relative = 1e-9
    );
    assert!(results.events.iter().any(|e| matches!(e, BacktestEvent::CorporateAction(c) if c.outcome == CorporateActionOutcome::Applied)));
    // The next day's price is the plain close too.
    let next = results.rows.iter().find(|r| r.date == day(29)).unwrap();
    approx::assert_relative_eq!(
        next.actual,
        100.0 * (closes[29] - closes[28]),
        max_relative = 1e-9
    );
}
