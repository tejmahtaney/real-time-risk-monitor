//! Shared fixtures for engine's own tests: a tiny hand-built two-symbol seed and
//! portfolio, fast enough and deterministic enough to drive directly, plus a
//! [`FailingFeed`] that can be told to fail on command, since `GbmSimulator` cannot be
//! made to fail deterministically.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use domain::{ClockStep, FixedClock, InstrumentKind, PositionId, SimClock, Symbol};
use engine::{
    Engine, FeedError, GbmSimulator, LimitsConfig, PositionSpec, PriceFeed, SeedData,
    SimulationConfig, VarMethod,
};
use marketdata::{DataSource, MarketTick, SymbolSeed, VolProcessConfig};
use portfolio::{ExpirySpec, StrikeSpec};
use risk::{PortfolioLimits, UnderlyingDeltaLimit};

/// Monday 28 September 2026, 09:30 EDT — a session open, matching `domain`'s own tests.
pub fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap()
}

/// A fast, fixed-seed simulation config: 3,600x speed, so a handful of real-seeming
/// ticks cover a whole session, and a real vol process so spot and vol genuinely move.
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

/// A hand-built two-symbol seed: no real market data or calibration involved.
pub fn seed_data() -> SeedData {
    let spy = Symbol::new("SPY").unwrap();
    let aapl = Symbol::new("AAPL").unwrap();
    SeedData {
        as_of: chrono::NaiveDate::from_ymd_opt(2026, 9, 25).unwrap(),
        rate: 0.04,
        symbols: vec![spy.clone(), aapl.clone()],
        underlyings: BTreeMap::from([
            (spy.clone(), symbol_seed(600.0, 0.20, 0.01)),
            (aapl.clone(), symbol_seed(230.0, 0.25, 0.005)),
        ]),
        return_dates: Vec::new(),
        returns: Vec::new(),
        correlation: vec![vec![1.0, 0.1], vec![0.1, 1.0]],
    }
}

/// Three positions: a stock, a put expiring today (to exercise settlement at the
/// close), and a call expiring in about a month.
pub fn positions() -> Vec<PositionSpec> {
    vec![
        PositionSpec {
            id: PositionId::new("spy-stock").unwrap(),
            underlying: Symbol::new("SPY").unwrap(),
            kind: InstrumentKind::Equity,
            strike: None,
            expiry: None,
            quantity: 10.0,
            entry_price: None,
            iv: None,
        },
        PositionSpec {
            id: PositionId::new("spy-put-0d").unwrap(),
            underlying: Symbol::new("SPY").unwrap(),
            kind: InstrumentKind::Put,
            strike: Some(StrikeSpec::Atm { offset_pct: 0.0 }),
            expiry: Some(ExpirySpec::Today),
            quantity: 2.0,
            entry_price: None,
            iv: None,
        },
        PositionSpec {
            id: PositionId::new("aapl-call-30d").unwrap(),
            underlying: Symbol::new("AAPL").unwrap(),
            kind: InstrumentKind::Call,
            strike: Some(StrikeSpec::Atm { offset_pct: 5.0 }),
            expiry: Some(ExpirySpec::FirstFridayAfter(30)),
            quantity: 3.0,
            entry_price: None,
            iv: None,
        },
    ]
}

/// Generous limits, so alert noise never distracts from what these tests actually check.
pub fn limits_config() -> LimitsConfig {
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
            overrides: BTreeMap::new(),
        },
        var_99: None,
    }
}

/// The RNG seed passed to `Engine::new` by every fixture here. A fixed literal, not
/// `sim_config().seed`, so a test can tell the two "seed" concepts apart at a glance.
pub const RNG_SEED: u64 = 42;

/// A `StorageRoot` inside a fresh temp directory, per CLAUDE.md's "tests never touch
/// the real storage folder" — the `TempDir` guard must outlive the root, so callers
/// that spawn an engine should keep it alive for the test's duration.
pub fn temp_storage() -> (tempfile::TempDir, engine::StorageRoot) {
    let dir = tempfile::tempdir().unwrap();
    let root = engine::StorageRoot::at(dir.path());
    (dir, root)
}

/// Builds the fixture engine over a real `GbmSimulator`.
pub fn build_engine() -> Engine<GbmSimulator> {
    let seed = seed_data();
    let config = sim_config();
    let feed = GbmSimulator::new(&seed, &config).unwrap();
    let clock = SimClock::new(start(), config.speed).unwrap();
    Engine::new(
        clock,
        &seed,
        feed,
        &positions(),
        limits_config(),
        Box::new(FixedClock::new(start())),
        100,
        RNG_SEED,
        VarMethod::DeltaGammaMc,
    )
    .unwrap()
}

/// Builds the fixture engine over a [`FailingFeed`], returning the flag that controls
/// whether its next `step` call fails.
pub fn build_failing_engine() -> (Engine<FailingFeed>, Arc<AtomicBool>) {
    let seed = seed_data();
    let config = sim_config();
    let inner = GbmSimulator::new(&seed, &config).unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let feed = FailingFeed::new(inner, fail.clone());
    let clock = SimClock::new(start(), config.speed).unwrap();
    let engine = Engine::new(
        clock,
        &seed,
        feed,
        &positions(),
        limits_config(),
        Box::new(FixedClock::new(start())),
        100,
        RNG_SEED,
        VarMethod::DeltaGammaMc,
    )
    .unwrap();
    (engine, fail)
}

/// A [`PriceFeed`] wrapping a real [`GbmSimulator`] that fails on command: `step`
/// returns [`FeedError::InvalidStep`] whenever `fail` is `true`, and delegates to the
/// inner simulator otherwise. `GbmSimulator` itself cannot be made to fail
/// deterministically, so this is how engine's own tests exercise the degraded-feed path.
pub struct FailingFeed {
    inner: GbmSimulator,
    fail: Arc<AtomicBool>,
}

impl FailingFeed {
    /// Wraps `inner`, sharing `fail` with the caller so it can flip it at will.
    pub fn new(inner: GbmSimulator, fail: Arc<AtomicBool>) -> Self {
        Self { inner, fail }
    }
}

impl PriceFeed for FailingFeed {
    fn step(&mut self, step: &ClockStep) -> Result<MarketTick, FeedError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(FeedError::InvalidStep("forced failure for testing".into()));
        }
        self.inner.step(step)
    }
}
