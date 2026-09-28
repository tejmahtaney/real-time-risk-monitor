//! Storage-facing endpoints end to end: saving and loading a portfolio version over
//! HTTP, path-traversal rejection, and that a restart with `start_portfolio = "last"`
//! restores `portfolios/current.json`.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use engine::{Engine, FixedClock, GbmSimulator, SimClock};
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
