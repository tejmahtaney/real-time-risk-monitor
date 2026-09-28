//! HTTP tests for every route, using `tower::ServiceExt::oneshot` so no real listener
//! is needed. Each test builds its own small, fast fixture engine (mirroring `engine`'s
//! own test fixture, duplicated here rather than shared across the crate boundary).
//!
//! `unwrap_used` is allowed for this whole file: clippy's `allow-unwrap-in-tests` only
//! exempts code textually inside a `#[test]`-annotated function, not the plain helper
//! functions below that the tests call into, even though this file only ever compiles
//! as a test binary; failing fast on a broken fixture or an unexpected response shape
//! is exactly what these tests want.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use engine::{
    DataSource, Engine, FixedClock, GbmSimulator, InstrumentKind, LimitsConfig, PositionId,
    PositionSpec, SeedData, SimClock, SimulationConfig, Symbol, SymbolSeed, VolProcessConfig,
};
use serde_json::{Value, json};
use server::{AppState, ServerConfig, build_router};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn start() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap()
}

fn sim_config() -> SimulationConfig {
    SimulationConfig {
        seed: 42,
        tick_interval_ms: 250,
        speed: 3_600.0,
        drift: 0.0,
        start_date: None,
        vol_process: VolProcessConfig {
            mean_reversion: 5.0,
            vol_of_vol: 0.9,
            spot_vol_correlation: -0.7,
        },
    }
}

fn symbol_seed(spot: f64, vol: f64, dividend_yield: f64) -> SymbolSeed {
    SymbolSeed {
        spot,
        vol_realised: vol,
        vol_ewma: vol,
        vol,
        dividend_yield,
        source: DataSource::Fallback {
            file: "test-fixture".to_owned(),
        },
    }
}

fn seed_data() -> SeedData {
    let spy = Symbol::new("SPY").unwrap();
    SeedData {
        as_of: chrono::NaiveDate::from_ymd_opt(2026, 9, 25).unwrap(),
        rate: 0.04,
        symbols: vec![spy.clone()],
        underlyings: BTreeMap::from([(spy, symbol_seed(600.0, 0.20, 0.01))]),
        return_dates: Vec::new(),
        returns: Vec::new(),
        correlation: vec![vec![1.0]],
    }
}

fn positions() -> Vec<PositionSpec> {
    vec![PositionSpec {
        id: PositionId::new("spy-stock").unwrap(),
        underlying: Symbol::new("SPY").unwrap(),
        kind: InstrumentKind::Equity,
        strike: None,
        expiry: None,
        quantity: 10.0,
        entry_price: None,
        iv: None,
    }]
}

fn server_config() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_owned(),
        port: 0,
        storage_dir: PathBuf::from("../storage"),
        static_dir: PathBuf::from("frontend/dist"),
        cors_origins: vec!["http://localhost:5173".to_owned()],
        market_config_path: repo_root().join("config/market.toml"),
        simulation_config_path: repo_root().join("config/simulation.toml"),
        limits_config_path: repo_root().join("config/limits.toml"),
        portfolio_path: repo_root().join("data/sample_portfolio.csv"),
    }
}

/// Builds a fresh router over a freshly spawned, fast fixture engine.
fn test_router() -> Router {
    let seed = seed_data();
    let config = sim_config();
    let feed = GbmSimulator::new(&seed, &config).unwrap();
    let clock = SimClock::new(start(), config.speed).unwrap();
    let limits = LimitsConfig::load(&repo_root().join("config/limits.toml")).unwrap();
    let engine = Engine::new(
        clock,
        &seed,
        feed,
        &positions(),
        limits,
        Box::new(FixedClock::new(start())),
    )
    .unwrap();
    let handle = engine.spawn(Duration::from_millis(20), CancellationToken::new());
    let state = AppState {
        engine: handle,
        config: Arc::new(server_config()),
        started_at: Instant::now(),
    };
    build_router(state)
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn json_request(method: &str, uri: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn health_reports_ok() {
    let response = test_router().oneshot(get("/api/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn positions_lists_the_starting_book() {
    let response = test_router().oneshot(get("/api/positions")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let positions = body.as_array().unwrap();
    assert!(positions.iter().any(|p| p["id"] == "spy-stock"));
}

#[tokio::test]
async fn greeks_and_risk_summary_and_pnl_and_alerts_respond() {
    let router = test_router();

    let response = router.clone().oneshot(get("/api/greeks")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["totals"].is_object());

    let response = router
        .clone()
        .oneshot(get("/api/risk-summary"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["compute_micros"].is_number());

    let response = router.clone().oneshot(get("/api/pnl")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["portfolio"].is_object());

    let response = router.oneshot(get("/api/alerts")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["active"].is_array());
    assert!(body["history"].is_array());
}

#[tokio::test]
async fn adding_and_removing_a_position_round_trips() {
    let router = test_router();

    let new_position = json!({
        "id": "aapl-stock",
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
    let body = json_body(response).await;
    assert_eq!(body["id"], "aapl-stock");

    // `GET /api/positions` reads the latest published snapshot, which only refreshes on
    // a tick; give the fixture's 20 ms interval a moment to fire.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = router.clone().oneshot(get("/api/positions")).await.unwrap();
    let body = json_body(response).await;
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "aapl-stock")
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/positions/aapl-stock")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn control_pauses_and_resumes() {
    let router = test_router();

    let response = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/control",
            &json!({"action": "pause"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = router
        .oneshot(json_request(
            "POST",
            "/api/control",
            &json!({"action": "resume"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn control_rejects_a_bad_speed_with_a_400_and_the_error_shape() {
    let response = test_router()
        .oneshot(json_request(
            "POST",
            "/api/control",
            &json!({"action": "set_speed", "speed": 9_999.0}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "validation_error");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn removing_an_unknown_position_is_a_404_with_the_error_shape() {
    let response = test_router()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/positions/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn cors_allows_the_configured_origin() {
    let request = Request::builder()
        .method("GET")
        .uri("/api/health")
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let response = test_router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_some(),
        "the configured origin should be echoed back"
    );
}
