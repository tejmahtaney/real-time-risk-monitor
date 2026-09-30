//! HTTP tests for every route, using `tower::ServiceExt::oneshot` so no real listener
//! is needed. Shared fixtures live in `tests/support/mod.rs`.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

mod support;
use support::{get, json_body, json_request, test_router};

#[tokio::test]
async fn health_reports_ok() {
    let (router, _dir) = test_router();
    let response = router.oneshot(get("/api/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    assert!(body["connected_ws_clients"].is_number());
    assert!(body["dropped_records"].is_number());
    assert_eq!(body["storage_warning"], false);
}

#[tokio::test]
async fn positions_lists_the_starting_book() {
    let (router, _dir) = test_router();
    let response = router.oneshot(get("/api/positions")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let positions = body.as_array().unwrap();
    assert!(positions.iter().any(|p| p["id"] == "spy-stock"));
}

#[tokio::test]
async fn greeks_and_risk_summary_and_pnl_and_alerts_respond() {
    let (router, _dir) = test_router();

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
async fn var_report_responds_with_sane_numbers() {
    let (router, _dir) = test_router();
    let response = router.oneshot(get("/api/var")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert!(body["headline_method"].is_string());
    for method in ["delta_normal", "delta_gamma_mc", "historical"] {
        let report = &body[method];
        let var_95 = report["var_95"].as_f64().unwrap();
        let var_99 = report["var_99"].as_f64().unwrap();
        let es_97_5 = report["es_97_5"].as_f64().unwrap();
        assert!(
            var_95 <= var_99,
            "{method}: var_95 {var_95} > var_99 {var_99}"
        );
        assert!(
            es_97_5 >= var_99,
            "{method}: es_97_5 {es_97_5} < var_99 {var_99}"
        );
        assert!(report["compute_micros"].is_number());
    }
    assert!(body["recompute_every_ticks"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn history_returns_points() {
    let (router, _dir) = test_router();

    // Give the fixture's 20 ms interval a moment to produce a few ticks.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = router.oneshot(get("/api/history")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let points = body.as_array().unwrap();
    assert!(!points.is_empty());
    assert!(points[0]["seq"].is_number());
    assert!(points[0]["worst_limit_utilisation"].is_number());
}

#[tokio::test]
async fn adding_and_removing_a_position_round_trips() {
    let (router, _dir) = test_router();

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
    let (router, _dir) = test_router();

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
    let (router, _dir) = test_router();
    let response = router
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
    let (router, _dir) = test_router();
    let response = router
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
    let (router, _dir) = test_router();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_some(),
        "the configured origin should be echoed back"
    );
}
