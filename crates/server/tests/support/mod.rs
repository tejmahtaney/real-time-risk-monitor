//! Shared fixtures for `server`'s own integration tests: a tiny hand-built one-symbol
//! seed and one-position portfolio, a fixture `Router`, and small HTTP helpers.
//!
//! Each `tests/*.rs` file compiles this module as part of its own separate binary and
//! only uses some of it, so `dead_code` is allowed here: it is not a sign of anything
//! actually unused in the crate as a whole. `unwrap_used` is also allowed: clippy's
//! `allow-unwrap-in-tests` only exempts code textually inside a `#[test]`-annotated
//! function, not plain helper functions like these that a test calls into, even though
//! this whole module only ever exists inside a `tests/` binary; failing fast on a
//! broken fixture is exactly what a test wants here.
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use chrono::{TimeZone, Utc};
use engine::{
    DataSource, Engine, FixedClock, GbmSimulator, InstrumentKind, LimitsConfig, PositionId,
    PositionSpec, SeedData, SimClock, SimulationConfig, Symbol, SymbolSeed, VolProcessConfig,
};
use serde_json::Value;
use server::{AppState, ServerConfig, build_router};
use tokio_util::sync::CancellationToken;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

pub fn start() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap()
}

pub fn sim_config() -> SimulationConfig {
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

pub fn symbol_seed(spot: f64, vol: f64, dividend_yield: f64) -> SymbolSeed {
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

pub fn seed_data() -> SeedData {
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

pub fn positions() -> Vec<PositionSpec> {
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

pub fn server_config() -> ServerConfig {
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
        history_capacity: 2_400,
    }
}

/// Builds a fresh router over a freshly spawned, fast fixture engine, using the real
/// `config/limits.toml` and a 20 ms tick interval. Use [`test_router_with`] directly
/// when a test needs its own `LimitsConfig` or tick rate (deterministically triggering
/// an alert, or controlling how fast snapshots change).
pub fn test_router() -> Router {
    let limits = LimitsConfig::load(&repo_root().join("config/limits.toml")).unwrap();
    test_router_with(limits, Duration::from_millis(20))
}

/// Builds a fresh router over a freshly spawned fixture engine with a caller-chosen
/// `LimitsConfig` and tick interval.
pub fn test_router_with(limits: LimitsConfig, tick_interval: Duration) -> Router {
    let seed = seed_data();
    let config = sim_config();
    let feed = GbmSimulator::new(&seed, &config).unwrap();
    let clock = SimClock::new(start(), config.speed).unwrap();
    let engine = Engine::new(
        clock,
        &seed,
        feed,
        &positions(),
        limits,
        Box::new(FixedClock::new(start())),
        2_400,
    )
    .unwrap();
    let handle = engine.spawn(tick_interval, CancellationToken::new());
    let state = AppState {
        engine: handle,
        config: Arc::new(server_config()),
        started_at: Instant::now(),
        #[allow(clippy::cast_possible_truncation)]
        tick_interval_ms: tick_interval.as_millis() as u64,
        ws_clients: Arc::new(AtomicUsize::new(0)),
    };
    build_router(state)
}

pub async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

pub fn json_request(method: &str, uri: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}
