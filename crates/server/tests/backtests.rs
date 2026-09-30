//! HTTP and WebSocket tests for the backtester's routes. Shared fixtures live in
//! `tests/support/mod.rs`; every run is saved under a temp storage folder.
#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as TMessage;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

mod support;
use support::{get, json_body, json_request, test_router};

const BOUNDARY: &str = "----rrm-test-boundary";

/// SPY closes alternating between two values, one line per day from 5 January 2026.
fn prices_csv(days: usize) -> String {
    let mut out = String::from("date,symbol,close\n");
    for i in 0..days {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
            + chrono::Duration::days(i64::try_from(i).unwrap());
        let close = if i % 2 == 0 { 100.0 } else { 101.0 };
        let _ = writeln!(out, "{date},SPY,{close}");
    }
    out
}

fn multipart_request(file: &[u8], fields: &[(&str, &str)]) -> Request<Body> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"prices.csv\"\r\nContent-Type: text/csv\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method("POST")
        .uri("/api/backtests")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("content-length", body.len())
        .body(Body::from(body))
        .unwrap()
}

async fn create(router: &Router, days: usize) -> String {
    let response = router
        .clone()
        .oneshot(multipart_request(prices_csv(days).as_bytes(), &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["id"].as_str().unwrap().to_owned()
}

async fn control(router: &Router, id: &str, body: &Value) -> axum::response::Response {
    router
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/backtests/{id}/control"),
            body,
        ))
        .await
        .unwrap()
}

async fn wait_finished(router: &Router, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let response = router
            .clone()
            .oneshot(get(&format!("/api/backtests/{id}")))
            .await
            .unwrap();
        if json_body(response).await["state"] == "finished" {
            return;
        }
        assert!(Instant::now() < deadline, "the run never finished");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn next<S>(ws: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<TMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let Some(Ok(TMessage::Text(text))) = ws.next().await else {
        unreachable!("expected a text message");
    };
    serde_json::from_str::<Value>(&text).unwrap()
}

fn delete_files_request(id: &str, body: Option<&Value>) -> Request<Body> {
    let builder = Request::builder()
        .method("DELETE")
        .uri(format!("/api/backtests/{id}/files"));
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn upload_run_and_read_the_results() {
    let (router, dir) = test_router();
    let id = create(&router, 45).await;

    let status = json_body(
        router
            .clone()
            .oneshot(get(&format!("/api/backtests/{id}")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["state"], "paused");
    assert_eq!(status["steps_total"], 24);
    assert_eq!(status["steps_done"], 0);

    let response = control(&router, &id, &json!({"action": "run_to_end"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    wait_finished(&router, &id).await;

    let results = json_body(
        router
            .clone()
            .oneshot(get(&format!("/api/backtests/{id}/results")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(results["rows"].as_array().unwrap().len(), 24);
    assert_eq!(results["partial"], false);
    assert_eq!(results["summary"]["days"], 24);
    assert!(results["summary"]["var_backtest"]["p_value"].is_number());
    assert_eq!(results["settings"]["warm_up_dates"], 20);

    let listed = json_body(router.clone().oneshot(get("/api/backtests")).await.unwrap()).await;
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["state"], "finished");

    // The run's files are on disk.
    let folder = dir.path().join("backtests").join(&id);
    for file in ["input.csv", "settings.json", "daily.csv", "summary.json"] {
        assert!(folder.join(file).exists(), "{file}");
    }
    assert_eq!(
        std::fs::read_to_string(folder.join("input.csv")).unwrap(),
        prices_csv(45)
    );
}

#[tokio::test]
async fn an_upload_over_five_megabytes_is_a_json_413() {
    let (router, dir) = test_router();
    let big = vec![b'x'; 6 * 1024 * 1024];
    let response = router
        .clone()
        .oneshot(multipart_request(&big, &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "payload_too_large");
    assert!(body["error"]["message"].as_str().unwrap().contains("5 MB"));
    // Nothing was created.
    assert!(!dir.path().join("backtests").exists());

    // A file just under the limit is read (and rejected as bad CSV, not as too large).
    let under = vec![b'x'; 5 * 1024 * 1024 - 1024];
    let response = router
        .oneshot(multipart_request(&under, &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn other_routes_keep_the_small_body_limit() {
    let (router, _dir) = test_router();
    let big = "x".repeat(70 * 1024);
    let request = Request::builder()
        .method("POST")
        .uri("/api/control")
        .header("content-type", "application/json")
        .body(Body::from(big))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_third_running_backtest_is_a_json_429() {
    let (router, _dir) = test_router();
    let first = create(&router, 45).await;
    let _second = create(&router, 45).await;
    let response = router
        .clone()
        .oneshot(multipart_request(prices_csv(45).as_bytes(), &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "too_many_requests");

    // Stopping one frees a place. Its files stay.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/backtests/{first}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    create(&router, 45).await;
    let saved = router
        .oneshot(get(&format!("/api/backtests/{first}")))
        .await
        .unwrap();
    assert_eq!(json_body(saved).await["state"], "unfinished");
}

#[tokio::test]
async fn bad_input_is_a_400_with_the_line_number() {
    let (router, _dir) = test_router();
    let mut csv = prices_csv(30);
    csv.push_str("2026-03-01,SPY,not-a-number\n");
    let response = router
        .clone()
        .oneshot(multipart_request(csv.as_bytes(), &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "validation_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("line 32:")
    );

    // No file, and a bad speed, are 400s too.
    let response = router
        .clone()
        .oneshot(multipart_request(b"", &[("speed", "500")]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // The archive source, with nothing archived in this temp storage.
    let response = router
        .oneshot(json_request(
            "POST",
            "/api/backtests",
            &json!({"source": "archive"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        json_body(response).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("archive")
    );
}

#[tokio::test]
async fn control_rejects_bad_commands_and_unknown_runs() {
    let (router, _dir) = test_router();
    let id = create(&router, 45).await;
    for bad in [
        json!({"action": "set_speed", "speed": 51}),
        json!({"action": "set_speed", "speed": 0}),
    ] {
        let response = control(&router, &id, &bad).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    let response = control(&router, &id, &json!({"action": "fly"})).await;
    assert!(response.status().is_client_error());
    let ok = control(&router, &id, &json!({"action": "set_speed", "speed": 50})).await;
    assert_eq!(json_body(ok).await["speed"], 50.0);

    let response = control(&router, "20260101T000000Z-nope", &json!({"action": "play"})).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = router
        .oneshot(get("/api/backtests/20260101T000000Z-nope/results"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(response).await["error"]["code"], "not_found");
}

#[tokio::test]
async fn deleting_files_needs_the_matching_confirm_body() {
    let (router, dir) = test_router();
    let id = create(&router, 45).await;
    let folder = dir.path().join("backtests").join(&id);
    assert!(folder.join("settings.json").exists());

    // No body, an empty object, the wrong id, and a stray sibling name: 400, and nothing is touched.
    for body in [
        None,
        Some(json!({})),
        Some(json!({"confirm": "yes"})),
        Some(json!({"confirm": "../sessions"})),
    ] {
        let response = router
            .clone()
            .oneshot(delete_files_request(&id, body.as_ref()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body:?}");
        assert_eq!(
            json_body(response).await["error"]["code"],
            "validation_error"
        );
        assert!(
            folder.join("settings.json").exists(),
            "{body:?} must delete nothing"
        );
    }
    // A confirm that names another run does not delete this one or that one.
    let other = create(&router, 45).await;
    let response = router
        .clone()
        .oneshot(delete_files_request(&id, Some(&json!({"confirm": other}))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(folder.join("settings.json").exists());
    assert!(
        dir.path()
            .join("backtests")
            .join(&other)
            .join("settings.json")
            .exists()
    );

    // The matching confirm deletes exactly this folder.
    let response = router
        .clone()
        .oneshot(delete_files_request(&id, Some(&json!({"confirm": id}))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!folder.exists());
    assert!(
        dir.path()
            .join("backtests")
            .join(&other)
            .join("settings.json")
            .exists()
    );
    let response = router
        .oneshot(delete_files_request(&id, Some(&json!({"confirm": id}))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_replay_streams_over_the_same_protocol_as_the_live_feed() {
    let (router, _dir) = test_router();
    let id = create(&router, 45).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let served = router.clone();
    tokio::spawn(async move {
        axum::serve(listener, served)
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await
            .unwrap();
    });

    // An unknown run is refused before the upgrade.
    let refused = connect_async(format!("ws://{addr}/ws/backtests/20260101T000000Z-nope")).await;
    assert!(refused.is_err());

    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/backtests/{id}"))
        .await
        .unwrap();
    let hello = next(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol_version"], 1);
    assert!((hello["speed"].as_f64().unwrap() - 5.0).abs() < 1e-12);
    let first = next(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["snapshot"]["seq"], 0);
    let start_date = first["snapshot"]["session_date"]
        .as_str()
        .unwrap()
        .to_owned();

    // Step once over HTTP; the client sees the next date's snapshot.
    control(&router, &id, &json!({"action": "step"})).await;
    let second = next(&mut ws).await;
    assert_eq!(second["type"], "snapshot");
    assert_eq!(second["snapshot"]["seq"], 1);
    assert_ne!(
        second["snapshot"]["session_date"].as_str().unwrap(),
        start_date
    );
    assert_eq!(second["snapshot"]["step_kind"], "overnight");

    // While a client is connected the run counts as in use; the health count includes it.
    let health = json_body(router.clone().oneshot(get("/api/health")).await.unwrap()).await;
    assert!(health["connected_ws_clients"].as_u64().unwrap() >= 1);
    shutdown.cancel();
}
