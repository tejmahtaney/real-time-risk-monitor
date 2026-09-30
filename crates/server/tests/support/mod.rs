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
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use chrono::{TimeZone, Utc};
use engine::{
    BacktestConfig, BacktestManager, CorporateActionsConfig, DataSource, Engine, FixedClock,
    GbmSimulator, InstrumentKind, LimitsConfig, MarketConfig, PositionId, PositionSpec, SeedData,
    SimClock, SimulationConfig, StorageRoot, Symbol, SymbolSeed, VarMethod, VolProcessConfig,
};
use serde_json::Value;
use server::{AppState, ServerConfig, StartPortfolio, StorageConfig, build_router};
use tokio_util::sync::CancellationToken;

/// The RNG seed passed to `Engine::new` by every fixture here — distinct from
/// `sim_config().seed`, since production code passes the same field for both
/// purposes but the two are conceptually different (see `engine::Engine::new`'s doc
/// comment).
pub const RNG_SEED: u64 = 42;

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
        dividend_schedule: std::collections::BTreeMap::default(),
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
        multiplier: None,
        adjusted: None,
        applied_actions: None,
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
        risk_config_path: repo_root().join("config/risk.toml"),
        storage_config_path: repo_root().join("config/storage.toml"),
        portfolio_path: repo_root().join("data/sample_portfolio.csv"),
        corporate_actions_path: repo_root().join("config/corporate_actions.toml"),
        history_capacity: 2_400,
    }
}

pub fn storage_config() -> StorageConfig {
    StorageConfig {
        start_portfolio: StartPortfolio::Sample,
        max_bytes: 21_474_836_480,
        warn_fraction: 0.8,
    }
}

/// A `StorageRoot` inside a fresh temp directory, per CLAUDE.md's "tests never touch
/// the real storage folder." The returned `TempDir` guard must be kept alive (bind it,
/// even to `_dir`, never `_`) for as long as anything might still write to the root —
/// in particular a spawned engine's session recorder task, which keeps writing after
/// `test_router_with` returns.
pub fn temp_storage() -> (tempfile::TempDir, StorageRoot) {
    let dir = tempfile::tempdir().unwrap();
    let root = StorageRoot::at(dir.path());
    (dir, root)
}

/// The one-position portfolio every backtest fixture starts from, written once to a file outside
/// the repo and outside any storage folder: a SPY stock row and a SPY call.
pub fn backtest_portfolio_path() -> PathBuf {
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::temp_dir().join(format!(
            "rrm-server-tests-backtest-portfolio-{}.csv",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
             stock,SPY,equity,,,100,,\n\
             call,SPY,call,ATM,+30d,2,,\n",
        )
        .unwrap();
        path
    })
    .clone()
}

/// A backtest config over the repo's real config files, the fixture portfolio and `storage_root`.
pub fn backtest_config(storage_root: &StorageRoot) -> BacktestConfig {
    let root = repo_root();
    BacktestConfig {
        storage: storage_root.clone(),
        market: MarketConfig::load(&root.join("config/market.toml")).unwrap(),
        limits: LimitsConfig::load(&root.join("config/limits.toml")).unwrap(),
        headline_var_method: VarMethod::DeltaGammaMc,
        corporate_actions: CorporateActionsConfig::load(
            &root.join("config/corporate_actions.toml"),
        )
        .unwrap(),
        portfolio_path: backtest_portfolio_path(),
        config_paths: vec![root.join("config/market.toml"), backtest_portfolio_path()],
        max_running: BacktestConfig::DEFAULT_MAX_RUNNING,
        idle_after: BacktestConfig::DEFAULT_IDLE_AFTER,
    }
}

/// Builds a fresh router over a freshly spawned, fast fixture engine, using the real
/// `config/limits.toml` and a 20 ms tick interval. Use [`test_router_with`] directly
/// when a test needs its own `LimitsConfig` or tick rate (deterministically triggering
/// an alert, or controlling how fast snapshots change). Returns the `TempDir` guard the
/// caller must keep alive for the router's whole lifetime.
pub fn test_router() -> (Router, tempfile::TempDir) {
    let limits = LimitsConfig::load(&repo_root().join("config/limits.toml")).unwrap();
    test_router_with(limits, Duration::from_millis(20))
}

/// Builds a fresh router over a freshly spawned fixture engine with a caller-chosen
/// `LimitsConfig` and tick interval, and its own temp storage root. Returns the
/// `TempDir` guard the caller must keep alive for the router's whole lifetime.
pub fn test_router_with(
    limits: LimitsConfig,
    tick_interval: Duration,
) -> (Router, tempfile::TempDir) {
    test_router_with_config(server_config(), limits, tick_interval)
}

/// As [`test_router_with`], but over a caller-chosen [`ServerConfig`] — in particular
/// for overriding `static_dir` in a static-file-serving test.
pub fn test_router_with_config(
    config: ServerConfig,
    limits: LimitsConfig,
    tick_interval: Duration,
) -> (Router, tempfile::TempDir) {
    let seed = seed_data();
    let sim = sim_config();
    let feed = GbmSimulator::new(&seed, &sim).unwrap();
    let clock = SimClock::new(start(), sim.speed).unwrap();
    let engine = Engine::new(
        clock,
        &seed,
        feed,
        &positions(),
        limits,
        Box::new(FixedClock::new(start())),
        2_400,
        RNG_SEED,
        VarMethod::DeltaGammaMc,
    )
    .unwrap();
    let (dir, storage_root) = temp_storage();
    let storage_root_for_backtests = storage_root.clone();
    let (handle, _join) = engine.spawn(
        tick_interval,
        storage_root.clone(),
        CancellationToken::new(),
    );
    let state = AppState {
        engine: handle,
        config: Arc::new(config),
        storage_config: Arc::new(storage_config()),
        storage_root,
        storage_warning: Arc::new(AtomicBool::new(false)),
        started_at: Instant::now(),
        #[allow(clippy::cast_possible_truncation)]
        tick_interval_ms: tick_interval.as_millis() as u64,
        ws_clients: Arc::new(AtomicUsize::new(0)),
        backtests: Arc::new(BacktestManager::new(backtest_config(
            &storage_root_for_backtests,
        ))),
    };
    (build_router(state), dir)
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
