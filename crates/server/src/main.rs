//! The `risk-monitor` binary: starts the server. Only this binary may use `anyhow`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use engine::{
    AlphaVantageSource, Archive, CorporateActionCalendar, CorporateActionState,
    CorporateActionsConfig, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL, Engine, GbmSimulator,
    HistoryLoader, LimitsConfig, MarketConfig, PositionSpec, RiskConfig, SimClock,
    SimulationConfig, StorageRoot, SystemClock, apply_ledger_to_seed, calibrate, load_csv,
    market_date,
};
use server::{AppState, ServerConfig, StartPortfolio, StorageConfig, build_router};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        return run_healthcheck();
    }

    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(ServerConfig::load().context("loading server config")?);
    tracing::info!(storage_dir = %config.storage_dir.display(), "storage path");

    let market = MarketConfig::load(&config.market_config_path).context("loading market config")?;
    let sim_config = SimulationConfig::load(&config.simulation_config_path)
        .context("loading simulation config")?;
    let limits = LimitsConfig::load(&config.limits_config_path).context("loading limits config")?;
    let risk_config = RiskConfig::load(&config.risk_config_path).context("loading risk config")?;
    let storage_config = Arc::new(
        StorageConfig::load(&config.storage_config_path).context("loading storage config")?,
    );
    let storage_root = StorageRoot::at(&config.storage_dir);

    let mut seed = load_seed(&market, &storage_root).await?;

    let start = sim_config.start_time(&seed);
    // Corporate actions come before the simulator and engine are built: the ledger is replayed
    // into the seed, so the market and the simulator start in the terms of the restored book.
    let actions = prepare_corporate_actions(
        &config,
        &market,
        &storage_config,
        &storage_root,
        &mut seed,
        start,
    )
    .await
    .context("preparing corporate actions")?;
    let clock = SimClock::new(start, sim_config.speed).context("building the simulated clock")?;
    let feed = GbmSimulator::new(&seed, &sim_config).context("building the price simulator")?;
    let specs = starting_specs(&config, &storage_config, &storage_root)
        .context("loading the starting portfolio")?;

    let engine = Engine::new_with_actions(
        clock,
        &seed,
        feed,
        &specs,
        limits,
        Box::new(SystemClock),
        config.history_capacity,
        sim_config.seed,
        risk_config.headline_var_method,
        actions,
    )
    .context("building the engine")?;

    // The two pieces of one-time startup I/O session construction used to do itself:
    // copy this session's config files, and save the (possibly just-loaded) starting
    // book as the automatic `session-start` version. Neither the engine struct nor its
    // tick-loop task ever touches disk — see `engine::recorder`'s doc comment.
    engine::copy_session_config(
        &storage_root,
        engine.session_id(),
        &session_config_paths(&config),
    )
    .context("copying this session's config files")?;
    save_session_start_version(&storage_root, &engine.active_specs())
        .context("saving the automatic session-start portfolio version")?;

    let shutdown = CancellationToken::new();
    let (handle, engine_task) = engine.spawn(
        sim_config.tick_interval(),
        storage_root.clone(),
        shutdown.clone(),
    );

    let storage_warning = Arc::new(AtomicBool::new(false));
    tokio::spawn(monitor_storage_usage(
        storage_root.clone(),
        storage_config.max_bytes,
        storage_config.warn_fraction,
        storage_warning.clone(),
        shutdown.clone(),
    ));

    let app_state = AppState {
        engine: handle,
        config: config.clone(),
        storage_config,
        storage_root,
        storage_warning,
        started_at: Instant::now(),
        tick_interval_ms: sim_config.tick_interval_ms,
        ws_clients: Arc::new(AtomicUsize::new(0)),
    };
    let router = build_router(app_state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await
        .context("serving")?;

    // Graceful shutdown above only means axum stopped accepting new work; the engine
    // task (and, inside it, the session recorder) reacts to the same cancellation
    // independently. Awaiting it here is what actually makes "stop the engine, drain,
    // flush, write the final meta.json, then exit" happen in that order before the
    // process ends.
    if let Err(err) = engine_task.await {
        tracing::error!(error = %err, "the engine task panicked during shutdown");
    }

    Ok(())
}

/// `risk-monitor --healthcheck`: a bare TCP `GET /api/health` against this same
/// process's own port on localhost, for Docker's `HEALTHCHECK` instruction (the
/// runtime image has no `curl`; see the Dockerfile). Exits 0 on any 2xx status line,
/// 1 otherwise, including a connection failure — `ServerConfig::load` failing counts
/// as a healthcheck failure too, via the `Err` this returns.
fn run_healthcheck() -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    let config = ServerConfig::load().context("loading server config for the healthcheck")?;
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));

    let healthy = (|| -> std::io::Result<bool> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(
            b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        Ok(response.starts_with(b"HTTP/1.1 2") || response.starts_with(b"HTTP/1.0 2"))
    })()
    .unwrap_or(false);

    std::process::exit(i32::from(!healthy));
}

/// Fetches (or reads from the archive/fallback) and calibrates the market seed data.
async fn load_seed(
    market: &engine::MarketConfig,
    storage_root: &StorageRoot,
) -> anyhow::Result<engine::SeedData> {
    let api_key = std::env::var("ALPHA_VANTAGE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty());
    let source = match api_key {
        Some(key) => Some(
            AlphaVantageSource::new(key, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL)
                .context("building the Alpha Vantage client")?,
        ),
        None => None,
    };
    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(storage_root.clone()),
        "data/fallback",
        market.cache_max_age_hours,
        source,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&market.symbols, market.data_mode)
        .await
        .context("loading market history")?;
    calibrate(&histories, market).context("calibrating seed")
}

/// The config files copied into each session's folder, so it is self-contained.
fn session_config_paths(config: &ServerConfig) -> Vec<&std::path::Path> {
    vec![
        config.market_config_path.as_path(),
        config.simulation_config_path.as_path(),
        config.limits_config_path.as_path(),
        config.risk_config_path.as_path(),
        config.storage_config_path.as_path(),
        config.portfolio_path.as_path(),
        config.corporate_actions_path.as_path(),
    ]
}

/// Loads the corporate-action calendar and the ledger, and puts the seed in the terms of the
/// book the session will start from.
///
/// The ledger is what has been applied to the persisted book. A `last` start (and a saved
/// version) reads it and replays it into `seed`: spots are divided by each split ratio and cut
/// by each dividend, and dividend amounts still to come are divided by the splits before them.
/// The calendar drops what the ledger already holds, so nothing fires twice. A `sample` start
/// ignores the ledger, because the sample book is unadjusted, and sets it aside under a
/// timestamped name (renamed, never deleted).
///
/// The optional Alpha Vantage loader adds real declared dividends for the symbols on the
/// discrete model, one request each, only when `[alpha_vantage] enabled = true`.
async fn prepare_corporate_actions(
    config: &ServerConfig,
    market: &MarketConfig,
    storage_config: &StorageConfig,
    storage_root: &StorageRoot,
    seed: &mut engine::SeedData,
    start: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<CorporateActionState> {
    let actions_config = CorporateActionsConfig::load(&config.corporate_actions_path)
        .context("loading the corporate actions config")?;
    let start_date = market_date(start);
    let mut calendar = CorporateActionCalendar::resolve(&actions_config, start_date)?;
    let discrete = market.discrete_symbols();

    if actions_config.alpha_vantage.enabled && !discrete.is_empty() {
        let key = std::env::var("ALPHA_VANTAGE_API_KEY")
            .ok()
            .filter(|key| !key.is_empty());
        if let Some(key) = key {
            let source = AlphaVantageSource::new(key, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL)
                .context("building the Alpha Vantage client")?;
            let found = engine::load_upcoming_dividends(
                &source,
                storage_root,
                &discrete,
                start_date,
                chrono::Utc::now(),
            )
            .await;
            tracing::info!(
                count = found.len(),
                "loaded upcoming real dividends from Alpha Vantage"
            );
            calendar.merge(found);
        } else {
            tracing::warn!(
                "corporate_actions.toml enables Alpha Vantage but ALPHA_VANTAGE_API_KEY is not set"
            );
        }
    }
    calendar.validate_against(market)?;

    let ledger = if storage_config.start_portfolio == StartPortfolio::Sample {
        if let Some(moved) = engine::archive_ledger(storage_root, chrono::Utc::now())
            .context("setting the old applied-actions ledger aside")?
        {
            tracing::info!(path = %moved.display(), "sample start: the applied-actions ledger was set aside");
        }
        Vec::new()
    } else {
        engine::read_ledger(storage_root).context("reading the applied-actions ledger")?
    };
    if !ledger.is_empty() {
        tracing::info!(
            applied = ledger.len(),
            "replaying the applied-actions ledger into the seed"
        );
    }
    apply_ledger_to_seed(seed, &ledger)?;
    calendar.without_applied(&ledger);
    seed.dividend_schedule = calendar.dividend_schedule(&discrete, &ledger);
    Ok(CorporateActionState { calendar, ledger })
}

/// Resolves the starting portfolio specs per `storage_config.start_portfolio`.
fn starting_specs(
    config: &ServerConfig,
    storage_config: &StorageConfig,
    storage_root: &StorageRoot,
) -> anyhow::Result<Vec<PositionSpec>> {
    match &storage_config.start_portfolio {
        StartPortfolio::Sample => {
            Ok(load_csv(&config.portfolio_path).context("loading the sample portfolio")?)
        }
        StartPortfolio::Last => {
            let current = storage_root
                .join(&["portfolios", "current.json"])
                .context("building the portfolios/current.json path")?;
            if current.exists() {
                let positions: Vec<engine::Position> =
                    engine::read_json(&current).context("reading portfolios/current.json")?;
                Ok(positions.iter().map(engine::to_spec).collect())
            } else {
                Ok(load_csv(&config.portfolio_path).context("loading the sample portfolio")?)
            }
        }
        StartPortfolio::Version(file) => {
            let path = storage_root
                .join(&["portfolios", "versions", file])
                .context("building the version path")?;
            Ok(load_csv(&path).context("loading the starting portfolio version")?)
        }
    }
}

/// Writes the starting book as the automatic `<start>-session-start.csv` version, the
/// same operation `POST /api/portfolio/versions` performs, done once at startup instead
/// of via a request.
fn save_session_start_version(
    storage_root: &StorageRoot,
    specs: &[PositionSpec],
) -> anyhow::Result<()> {
    let bytes = engine::to_csv_bytes(specs)?;
    let dir = storage_root.subdir(&["portfolios", "versions"])?;
    engine::write_bytes_new(&dir, "session-start", "csv", chrono::Utc::now(), &bytes)?;
    Ok(())
}

/// Runs until `shutdown` fires. Every minute, measures usage by top-level folder;
/// above `warn_fraction` of `max_bytes`, logs a warning and sets `warning`. Never
/// deletes anything — only measures and flags.
async fn monitor_storage_usage(
    root: StorageRoot,
    max_bytes: u64,
    warn_fraction: f64,
    warning: Arc<AtomicBool>,
    shutdown: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = interval.tick() => {
                match engine::usage_by_folder(&root) {
                    Ok(usage) => {
                        let total: u64 = usage.values().sum();
                        let over = (total as f64) >= (max_bytes as f64) * warn_fraction;
                        warning.store(over, Ordering::Relaxed);
                        if over {
                            tracing::warn!(total, max_bytes, "storage usage above warn_fraction");
                        }
                    }
                    Err(err) => tracing::error!(error = %err, "failed to measure storage usage"),
                }
            }
        }
    }
}

/// Resolves on Ctrl-C or, on Unix, `SIGTERM` (which Docker sends on shutdown), then
/// cancels `token`, which stops the engine task too.
async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to install a SIGTERM handler");
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    token.cancel();
}
