//! Integration tests for `GET /ws`, against a real `TcpListener` bound to port 0 (a
//! WebSocket upgrade needs a genuine duplex byte stream, which
//! `tower::ServiceExt::oneshot` — used by `tests/http.rs` — cannot provide).
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::Router;
use engine::{LimitsConfig, PortfolioLimits, UnderlyingDeltaLimit};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as TMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tokio_util::sync::CancellationToken;

mod support;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Binds a real listener on `127.0.0.1:0`, serves `router` on it with graceful
/// shutdown, and returns the `ws://` URL for `/ws`, the plain `http://` base, and the
/// token that stops the server when cancelled.
async fn spawn_test_server(router: Router) -> (String, String, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let shutdown_for_task = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown_for_task.cancelled().await })
            .await
            .unwrap();
    });
    (
        format!("ws://{addr}/ws"),
        format!("http://{addr}"),
        shutdown,
    )
}

/// Reads the next frame and parses it as JSON. Panics (via `unreachable!`, since
/// `clippy::panic` is denied crate-wide) on anything but a text frame.
async fn read_json(ws: &mut WsStream) -> serde_json::Value {
    let Some(Ok(TMessage::Text(text))) = ws.next().await else {
        unreachable!("expected a text message");
    };
    serde_json::from_str(&text).unwrap()
}

/// Limits comfortably above anything the one-position fixture book can reach, so
/// tests that only care about the streaming mechanics see no alert noise.
fn generous_limits() -> LimitsConfig {
    LimitsConfig {
        warning_threshold: 0.8,
        warning_clear_threshold: 0.75,
        clear_threshold: 0.95,
        portfolio: PortfolioLimits {
            dollar_delta: 10_000_000.0,
            dollar_gamma_1pct: 10_000_000.0,
            vega: 10_000_000.0,
            theta_floor: 10_000_000.0,
            gross_exposure: 10_000_000.0,
            max_concentration: 1.0,
        },
        underlying_dollar_delta: UnderlyingDeltaLimit {
            default: 10_000_000.0,
            overrides: std::collections::BTreeMap::new(),
        },
        var_99: None,
    }
}

/// A portfolio dollar delta limit set just above the fixture's starting 10-share SPY
/// position (~$6,000), so it is `Ok` at connect time but easily breached by one more
/// sizeable position added mid-test.
fn tight_limits() -> LimitsConfig {
    LimitsConfig {
        portfolio: PortfolioLimits {
            dollar_delta: 10_000.0,
            ..generous_limits().portfolio
        },
        ..generous_limits()
    }
}

#[tokio::test]
async fn hello_then_snapshot_then_a_newer_snapshot_on_tick() {
    let router = support::test_router_with(generous_limits(), Duration::from_millis(20));
    let (ws_url, _http_base, shutdown) = spawn_test_server(router).await;

    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    let hello = read_json(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol_version"], 1);
    assert!(hello["tick_interval_ms"].is_number());
    assert!(hello["speed"].is_number());

    let first = read_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    let seq1 = first["snapshot"]["seq"].as_u64().unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let second = read_json(&mut ws).await;
    assert_eq!(second["type"], "snapshot");
    let seq2 = second["snapshot"]["seq"].as_u64().unwrap();
    assert!(
        seq2 > seq1,
        "expected a newer snapshot, got seq {seq1} then {seq2}"
    );

    shutdown.cancel();
}

/// Secondary, end-to-end sanity check for "the engine never waits for a client": the
/// precise version of this claim lives in `crates/engine/tests/streaming_backpressure.rs`,
/// which measures per-tick compute time directly. This test only checks that the
/// whole server, REST routes included, keeps responding promptly with a connected
/// client that never reads another frame after the handshake.
#[tokio::test]
async fn an_idle_websocket_client_does_not_stop_the_server_responding() {
    let router = support::test_router_with(generous_limits(), Duration::from_millis(2));
    let (ws_url, http_base, shutdown) = spawn_test_server(router).await;

    let (_idle_ws, _) = connect_async(&ws_url).await.unwrap();

    let client = reqwest::Client::new();
    let mut last_sim_time: Option<String> = None;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        let response = client
            .get(format!("{http_base}/api/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["connected_ws_clients"], 1);
        let sim_time = body["sim_time"].as_str().unwrap().to_owned();
        if let Some(previous) = &last_sim_time {
            assert_ne!(&sim_time, previous, "sim time should keep advancing");
        }
        last_sim_time = Some(sim_time);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn throttling_caps_the_snapshot_rate() {
    let router = support::test_router_with(generous_limits(), Duration::from_millis(5));
    let (ws_url, _http_base, shutdown) = spawn_test_server(router).await;

    let (mut ws, _) = connect_async(&ws_url).await.unwrap();
    let _hello = read_json(&mut ws).await;
    let _first_snapshot = read_json(&mut ws).await;

    let subscribe = json!({"type": "subscribe", "max_hz": 2.0}).to_string();
    ws.send(TMessage::Text(subscribe.into())).await.unwrap();

    let mut count = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_500);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(TMessage::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "snapshot" {
                    count += 1;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }

    assert!(
        (2..=4).contains(&count),
        "expected 2 to 4 throttled snapshots in 1.5s, got {count}"
    );

    shutdown.cancel();
}

#[tokio::test]
async fn an_alert_reaches_every_connected_client() {
    let router = support::test_router_with(tight_limits(), Duration::from_millis(20));
    let (ws_url, http_base, shutdown) = spawn_test_server(router).await;

    let (mut ws_a, _) = connect_async(&ws_url).await.unwrap();
    let (mut ws_b, _) = connect_async(&ws_url).await.unwrap();
    for ws in [&mut ws_a, &mut ws_b] {
        let _hello = read_json(ws).await;
        let _first_snapshot = read_json(ws).await;
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{http_base}/api/positions"))
        .json(&json!({
            "id": "spy-stock-2",
            "underlying": "SPY",
            "kind": "equity",
            "strike": null,
            "expiry": null,
            "quantity": 1_000.0,
            "entry_price": null,
            "iv": null,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    for ws in [&mut ws_a, &mut ws_b] {
        let mut saw_alert = false;
        for _ in 0..20 {
            let value = tokio::time::timeout(Duration::from_millis(500), read_json(ws))
                .await
                .unwrap();
            if value["type"] == "alert" {
                assert_eq!(value["alert"]["name"], "portfolio_dollar_delta");
                assert_eq!(value["alert"]["to"], "breach");
                saw_alert = true;
                break;
            }
        }
        assert!(saw_alert, "expected an alert message on this connection");
    }

    shutdown.cancel();
}
