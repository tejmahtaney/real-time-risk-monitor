//! Long-running soak tests. Both are `#[ignore]`d, so `cargo test` skips them.
//!
//! Run them on their own, in release mode, one at a time:
//!
//! ```text
//! cargo test --release -p server --test soak -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `max_speed_ten_minutes_twenty_clients` takes about 10 minutes and
//! `default_speed_never_drops_records` about 2. Each prints its measurements, which the
//! notes in `docs/notes/part-24-release.md` quote.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::time::{Duration, Instant};

use axum::Router;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tokio_util::sync::CancellationToken;

mod support;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

const CLIENTS: usize = 20;

async fn spawn_test_server(router: Router) -> (String, String, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await
            .unwrap();
    });
    (
        format!("ws://{addr}/ws"),
        format!("http://{addr}"),
        shutdown,
    )
}

/// Resident set size of this process in kibibytes, from `/proc/self/status` (Linux only).
fn rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap()
}

/// Total size of every file under `dir`, in bytes.
fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// Connects `CLIENTS` clients and keeps reading them, as fast as they can, until `stop`.
async fn spawn_clients(ws_url: &str, stop: CancellationToken) {
    for _ in 0..CLIENTS {
        let (mut ws, _): (WsStream, _) = connect_async(ws_url).await.unwrap();
        let stop = stop.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = stop.cancelled() => break,
                    frame = ws.next() => if frame.is_none() { break },
                }
            }
        });
    }
}

async fn health(base: &str) -> serde_json::Value {
    reqwest::get(format!("{base}/api/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Runs the engine with a `tick` interval for `duration` with `CLIENTS` readers and prints
/// RSS and storage growth once a minute. Returns `(rss_after_warmup, rss_at_end, dropped)`.
async fn soak(tick: Duration, duration: Duration) -> (u64, u64, u64) {
    let limits =
        engine::LimitsConfig::load(&support::repo_root().join("config/limits.toml")).unwrap();
    let (router, dir) = support::test_router_with(limits, tick);
    let (ws_url, base, shutdown) = spawn_test_server(router).await;
    let stop = CancellationToken::new();
    spawn_clients(&ws_url, stop.clone()).await;

    let started = Instant::now();
    let warmup = Duration::from_secs(60).min(duration / 5);
    tokio::time::sleep(warmup).await;
    let rss_warm = rss_kib();
    let bytes_warm = dir_bytes(dir.path());
    let warm_at = started.elapsed();
    let mut minute = 0;
    while started.elapsed() < duration {
        tokio::time::sleep(Duration::from_secs(60).min(duration.saturating_sub(started.elapsed())))
            .await;
        minute += 1;
        let h = health(&base).await;
        println!(
            "minute {minute}: rss {} KiB, storage {} bytes, ws clients {}, dropped {}",
            rss_kib(),
            dir_bytes(dir.path()),
            h["connected_ws_clients"],
            h["dropped_records"],
        );
    }
    let rss_end = rss_kib();
    let secs = started
        .elapsed()
        .saturating_sub(warm_at)
        .as_secs_f64()
        .max(1.0);
    #[allow(clippy::cast_precision_loss)]
    let growth = (dir_bytes(dir.path()) - bytes_warm) as f64 / secs;
    let dropped = health(&base).await["dropped_records"].as_u64().unwrap();
    println!(
        "storage growth {growth:.0} bytes/s ({:.1} MiB/hour), dropped records {dropped}",
        growth * 3600.0 / (1024.0 * 1024.0)
    );
    stop.cancel();
    shutdown.cancel();
    (rss_warm, rss_end, dropped)
}

/// Maximum speed (1 ms ticks), 20 clients, 10 minutes. Memory must stay flat: resident size at
/// the end within 25 % of its size after the one-minute warm-up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "takes 10 minutes; see the module doc for the command"]
async fn max_speed_ten_minutes_twenty_clients() {
    let (warm, end, _dropped) = soak(Duration::from_millis(1), Duration::from_secs(600)).await;
    println!("rss after warm-up {warm} KiB, at end {end} KiB");
    assert!(
        end * 4 <= warm * 5,
        "rss grew from {warm} KiB to {end} KiB, which is more than 25 %"
    );
}

/// The default 250 ms tick with 20 clients for 2 minutes: the recorder must drop nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "takes 2 minutes; see the module doc for the command"]
async fn default_speed_never_drops_records() {
    let (_warm, _end, dropped) = soak(Duration::from_millis(250), Duration::from_secs(120)).await;
    assert_eq!(dropped, 0, "the recorder dropped records at default speed");
}
