//! The `risk-monitor` binary: starts the server. Only this binary may use `anyhow`.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use engine::{
    AlphaVantageSource, Archive, DEFAULT_BASE_URL, DEFAULT_REQUEST_INTERVAL, Engine, GbmSimulator,
    HistoryLoader, LimitsConfig, MarketConfig, SimClock, SimulationConfig, StorageRoot,
    SystemClock, calibrate, load_csv,
};
use server::{AppState, ServerConfig, build_router};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        Archive::new(StorageRoot::at(&config.storage_dir)),
        "data/fallback",
        market.cache_max_age_hours,
        source,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&market.symbols, market.data_mode)
        .await
        .context("loading market history")?;
    let seed = calibrate(&histories, &market).context("calibrating seed")?;

    let start = sim_config.start_time(&seed);
    let clock = SimClock::new(start, sim_config.speed).context("building the simulated clock")?;
    let feed = GbmSimulator::new(&seed, &sim_config).context("building the price simulator")?;
    let specs = load_csv(&config.portfolio_path).context("loading the starting portfolio")?;

    let engine = Engine::new(clock, &seed, feed, &specs, limits, Box::new(SystemClock))
        .context("building the engine")?;

    let shutdown = CancellationToken::new();
    let handle = engine.spawn(sim_config.tick_interval(), shutdown.clone());

    let app_state = AppState {
        engine: handle,
        config: config.clone(),
        started_at: Instant::now(),
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

    Ok(())
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
