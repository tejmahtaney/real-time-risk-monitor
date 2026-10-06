//! Storage-facing endpoints end to end: saving and loading a portfolio version over
//! HTTP, path-traversal rejection, and that a restart with `start_portfolio = "last"`
//! restores `portfolios/current.json`.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use engine::{Engine, FixedClock, GbmSimulator, SimClock, VarMethod};
use serde_json::json;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

mod support;
use support::{get, json_body, json_request, test_router};

#[tokio::test]
async fn save_then_load_a_portfolio_version_restores_the_book_over_http() {
    let (router, _dir) = test_router();

    let response = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/portfolio/versions",
            &json!({"name": "before add"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let file = body["file"].as_str().unwrap().to_owned();
    assert!(file.ends_with("-before-add.csv"), "got {file:?}");

    let new_position = json!({
        "id": "spy-extra",
        "underlying": "SPY",
        "kind": "equity",
        "strike": null,
        "expiry": null,
        "quantity": 5.0,
        "entry_price": null,
        "iv": null,
    });
    let response = router
        .clone()
        .oneshot(json_request("POST", "/api/positions", &new_position))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    sleep(Duration::from_millis(100)).await;
    let response = router.clone().oneshot(get("/api/positions")).await.unwrap();
    let body = json_body(response).await;
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "spy-extra"),
        "the added position should be in the book before loading the version back"
    );

    let response = router
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/portfolio/versions/{file}/load"),
            &json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["position_count"], 1);

    sleep(Duration::from_millis(100)).await;
    let response = router.oneshot(get("/api/positions")).await.unwrap();
    let body = json_body(response).await;
    let positions = body.as_array().unwrap();
    assert_eq!(
        positions.len(),
        1,
        "the loaded version should replace the book: {positions:?}"
    );
    assert_eq!(positions[0]["id"], "spy-stock");
}

#[tokio::test]
async fn path_traversal_and_absolute_segments_are_rejected_with_400() {
    let (router, _dir) = test_router();

    let bad_requests = [
        ("GET", "/api/sessions/..%2Fetc"),
        ("GET", "/api/sessions/%2E%2E"),
        ("GET", "/api/sessions/%2Fetc%2Fpasswd/history"),
        ("GET", "/api/sessions/..%2Fetc/events"),
        ("GET", "/api/portfolio/versions/..%2F..%2Fetc%2Fpasswd"),
        ("POST", "/api/portfolio/versions/%2E%2E/load"),
    ];

    for (method, uri) in bad_requests {
        let body = if method == "POST" {
            json!({})
        } else {
            json!(null)
        };
        let request = if method == "POST" {
            json_request(method, uri, &body)
        } else {
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap()
        };
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{method} {uri} should be rejected"
        );
        let parsed = json_body(response).await;
        assert_eq!(
            parsed["error"]["code"], "validation_error",
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn restart_with_last_restores_the_book_from_current_json() {
    let (dir, storage) = support::temp_storage();
    let seed = support::seed_data();
    let sim_config = support::sim_config();
    let feed = GbmSimulator::new(&seed, &sim_config).unwrap();
    let clock = SimClock::new(support::start(), sim_config.speed).unwrap();
    let engine = Engine::new(
        clock,
        &seed,
        feed,
        &support::positions(),
        engine::LimitsConfig::load(&support::repo_root().join("config/limits.toml")).unwrap(),
        Box::new(FixedClock::new(support::start())),
        2_400,
        support::RNG_SEED,
        VarMethod::DeltaGammaMc,
    )
    .unwrap();
    let shutdown = CancellationToken::new();
    let (handle, join) = engine.spawn(Duration::from_millis(10), storage.clone(), shutdown.clone());

    // The fixture seed only has SPY, so the added position is a second SPY holding
    // rather than a new underlying.
    handle
        .send(engine::EngineCommand::AddPosition(engine::PositionSpec {
            id: engine::PositionId::new("spy-extra").unwrap(),
            underlying: engine::Symbol::new("SPY").unwrap(),
            kind: engine::InstrumentKind::Equity,
            strike: None,
            expiry: None,
            quantity: 5.0,
            entry_price: None,
            iv: None,
            multiplier: None,
            adjusted: None,
            applied_actions: None,
        }))
        .await
        .unwrap()
        .unwrap();

    // Shut down cleanly so the recorder's final flush is guaranteed before reading
    // current.json back.
    shutdown.cancel();
    join.await.unwrap();

    // The `StartPortfolio::Last` branch `main.rs` runs: read `current.json`, convert
    // each resolved `Position` back to a `PositionSpec` with `to_spec`.
    let current_path = storage.join(&["portfolios", "current.json"]).unwrap();
    let positions: Vec<engine::Position> = engine::read_json(&current_path).unwrap();
    let specs: Vec<engine::PositionSpec> = positions.iter().map(engine::to_spec).collect();
    assert_eq!(specs.len(), 2, "the original position plus the added one");
    assert!(specs.iter().any(|s| s.id.as_str() == "spy-stock"));
    assert!(specs.iter().any(|s| s.id.as_str() == "spy-extra"));

    let seed = support::seed_data();
    let feed = GbmSimulator::new(&seed, &sim_config).unwrap();
    let clock = SimClock::new(support::start(), sim_config.speed).unwrap();
    let restarted = Engine::new(
        clock,
        &seed,
        feed,
        &specs,
        engine::LimitsConfig::load(&support::repo_root().join("config/limits.toml")).unwrap(),
        Box::new(FixedClock::new(support::start())),
        2_400,
        support::RNG_SEED,
        VarMethod::DeltaGammaMc,
    )
    .unwrap();
    let restarted_ids: Vec<String> = restarted
        .active_specs()
        .iter()
        .map(|s| s.id.as_str().to_owned())
        .collect();
    assert!(restarted_ids.contains(&"spy-extra".to_owned()));
    assert!(restarted_ids.contains(&"spy-stock".to_owned()));

    drop(dir);
}

fn archive_file(storage: &std::path::Path, symbol: &str, name: &str, closes: &[(&str, f64)]) {
    let series: Vec<String> = closes
        .iter()
        .map(|(date, close)| {
            format!(
                r#""{date}":{{"1. open":"1","2. high":"1","3. low":"1","4. close":"{close}","5. volume":"1"}}"#
            )
        })
        .collect();
    let body = format!(
        r#"{{"Meta Data":{{"1. Information":"Daily Prices"}},"Time Series (Daily)":{{{}}}}}"#,
        series.join(",")
    );
    let record = json!({
        "symbol": symbol,
        "fetched_at": "2026-03-01T00:00:00Z",
        "params": {"function": "TIME_SERIES_DAILY", "symbol": symbol, "outputsize": "compact", "datatype": "json"},
        "body": body,
    });
    let folder = storage.join("market-data").join(symbol);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(name), record.to_string()).unwrap();
}

#[tokio::test]
async fn versions_list_carries_time_name_and_auto_flag_and_specs_come_back_as_json() {
    let (router, dir) = test_router();
    let folder = dir.path().join("portfolios").join("versions");
    std::fs::create_dir_all(&folder).unwrap();

    let response = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/portfolio/versions",
            &json!({"name": "Before Add"}),
        ))
        .await
        .unwrap();
    let file = json_body(response).await["file"]
        .as_str()
        .unwrap()
        .to_owned();
    let auto = std::fs::read(folder.join(&file)).unwrap();
    std::fs::write(folder.join("20260101T000000Z-session-start.csv"), auto).unwrap();

    let list = json_body(
        router
            .clone()
            .oneshot(get("/api/portfolio/versions"))
            .await
            .unwrap(),
    )
    .await;
    let rows = list.as_array().unwrap();
    let named = rows.iter().find(|r| r["file"] == file.as_str()).unwrap();
    assert_eq!(named["name"], "before add");
    assert_eq!(named["auto"], false);
    assert!(named["saved_at"].as_str().unwrap().ends_with('Z'));
    let auto_row = rows
        .iter()
        .find(|r| r["file"] == "20260101T000000Z-session-start.csv")
        .unwrap();
    assert_eq!(auto_row["auto"], true);
    assert_eq!(auto_row["saved_at"], "2026-01-01T00:00:00Z");

    let response = router
        .clone()
        .oneshot(get(&format!("/api/portfolio/versions/{file}/specs")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let specs = json_body(response).await;
    assert_eq!(specs.as_array().unwrap().len(), 1);
    assert_eq!(specs[0]["id"], "spy-stock");

    let response = router
        .oneshot(get("/api/portfolio/versions/%2E%2E/specs"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn archive_list_has_source_and_date_count_and_closes_come_from_their_own_route() {
    let (router, dir) = test_router();
    archive_file(
        dir.path(),
        "SPY",
        "20260301T000000Z-daily.json",
        &[("2026-02-26", 500.0), ("2026-02-27", 502.5)],
    );
    std::fs::write(
        dir.path()
            .join("market-data/SPY/20260302T000000Z-daily.json"),
        "not json",
    )
    .unwrap();

    let list = json_body(
        router
            .clone()
            .oneshot(get("/api/market-data/archive"))
            .await
            .unwrap(),
    )
    .await;
    let files = list[0]["files"].as_array().unwrap();
    assert_eq!(files[0]["source"], "Alpha Vantage TIME_SERIES_DAILY");
    assert_eq!(files[0]["dates"], 2);
    assert!(files[0].get("closes").is_none(), "the list stays light");
    assert_eq!(files[1]["source"], "unreadable");
    assert_eq!(files[1]["dates"], 0);

    let closes = json_body(
        router
            .clone()
            .oneshot(get(
                "/api/market-data/archive/SPY/20260301T000000Z-daily.json/closes",
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        closes,
        json!([
            {"date": "2026-02-26", "close": 500.0},
            {"date": "2026-02-27", "close": 502.5},
        ])
    );

    let missing = router
        .clone()
        .oneshot(get("/api/market-data/archive/SPY/nope.json/closes"))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let traversal = router
        .oneshot(get("/api/market-data/archive/%2E%2E/x.json/closes"))
        .await
        .unwrap();
    assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn archive_coverage_lists_each_symbols_merged_dates() {
    let (router, dir) = test_router();
    archive_file(
        dir.path(),
        "SPY",
        "20260301T000000Z-daily.json",
        &[("2026-02-25", 1.0), ("2026-02-26", 2.0)],
    );
    archive_file(
        dir.path(),
        "SPY",
        "20260302T000000Z-daily.json",
        &[("2026-02-26", 2.0), ("2026-02-27", 3.0)],
    );
    let coverage = json_body(
        router
            .oneshot(get("/api/backtests/archive-coverage"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        coverage["symbols"],
        json!([{"symbol": "SPY", "first_date": "2026-02-25", "last_date": "2026-02-27", "dates": 3}])
    );
}

#[tokio::test]
async fn one_unreadable_session_does_not_hide_the_others() {
    let (router, dir) = test_router();
    let meta = |id: &str, end: serde_json::Value| {
        let folder = dir.path().join("sessions").join(id);
        std::fs::create_dir_all(folder.join("closes")).unwrap();
        std::fs::write(
            folder.join("meta.json"),
            json!({
                "session_id": id, "start": "2026-09-28T12:00:00Z", "end": end, "seed": 1,
                "speed": 1.0, "git_commit": "abcdef1234", "dropped_records": 0
            })
            .to_string(),
        )
        .unwrap();
        folder
    };
    // An old build's close file, missing fields the current snapshot needs.
    let old = meta("old", json!(null));
    std::fs::write(old.join("closes/2026-09-28.json"), r#"{"seq": 1}"#).unwrap();
    std::fs::write(
        old.join("daily-pnl.csv"),
        "session_date,delta,gamma,vega,theta,rho,higher_order,dividends,residual,actual\n\
         2026-09-28,0,0,0,0,0,0,0,0,10.5\n2026-09-29,0,0,0,0,0,0,0,0,-2.5\n",
    )
    .unwrap();
    // A session whose meta is damaged.
    let broken = dir.path().join("sessions/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("meta.json"), "{").unwrap();

    let response = router.oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let list = json_body(response).await;
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "old");
    assert_eq!(rows[0]["simulated_days"], 1);
    // The close does not parse, so the total is the sum of the daily P&L file.
    assert_eq!(rows[0]["total_pnl"], 8.0);
    assert_eq!(rows[0]["end"], serde_json::Value::Null);
}

#[tokio::test]
async fn only_the_engines_current_session_is_running() {
    let (router, dir) = test_router();
    // An earlier session that never wrote an end time: cut short, not running.
    let folder = dir.path().join("sessions/20200101T000000Z-aaaa");
    std::fs::create_dir_all(folder.join("closes")).unwrap();
    std::fs::write(
        folder.join("meta.json"),
        json!({
            "session_id": "20200101T000000Z-aaaa", "start": "2020-01-01T00:00:00Z", "end": null,
            "seed": 1, "speed": 1.0, "git_commit": "abcdef1234", "dropped_records": 0
        })
        .to_string(),
    )
    .unwrap();
    // The recorder writes the engine's own meta.json once it starts.
    sleep(Duration::from_millis(300)).await;

    let response = router.clone().oneshot(get("/api/sessions")).await.unwrap();
    let list = json_body(response).await;
    let rows = list.as_array().unwrap();
    let running: Vec<_> = rows.iter().filter(|row| row["running"] == true).collect();
    assert_eq!(running.len(), 1, "{rows:?}");
    assert_ne!(running[0]["id"], "20200101T000000Z-aaaa");
    let old = rows
        .iter()
        .find(|row| row["id"] == "20200101T000000Z-aaaa")
        .unwrap();
    assert_eq!(old["running"], false);

    for (id, expected) in [
        (running[0]["id"].as_str().unwrap(), true),
        ("20200101T000000Z-aaaa", false),
    ] {
        let response = router
            .clone()
            .oneshot(get(&format!("/api/sessions/{id}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["running"], expected, "{id}");
    }
}
