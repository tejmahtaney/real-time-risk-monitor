//! Server configuration, layered with `figment`.

use std::path::PathBuf;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::Deserialize;
use thiserror::Error;

/// Settings for the server binary.
///
/// Loaded with [`ServerConfig::load`] from three layers, each overriding the last:
/// `config/server.toml`, then environment variables prefixed `RRM_`, then a bare `PORT`
/// variable for the port, which a later cloud host will need.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address. Defaults to `127.0.0.1` so the app is reachable only from this
    /// machine; only Docker overrides it, inside the container.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Root of the runtime storage folder. Also read directly by
    /// `storage::StorageRoot`; kept here so it can be logged at startup.
    pub storage_dir: PathBuf,
    /// Root the frontend's built static files are served from. Unused before Part 14.
    pub static_dir: PathBuf,
    /// Origins the CORS layer allows, such as `http://localhost:5173` for Vite.
    pub cors_origins: Vec<String>,
    /// Path to `config/market.toml`.
    pub market_config_path: PathBuf,
    /// Path to `config/simulation.toml`.
    pub simulation_config_path: PathBuf,
    /// Path to `config/limits.toml`.
    pub limits_config_path: PathBuf,
    /// Path to `config/storage.toml`.
    pub storage_config_path: PathBuf,
    /// Path to the starting portfolio CSV.
    pub portfolio_path: PathBuf,
    /// Capacity of the engine's history ring buffer, in points.
    pub history_capacity: usize,
}

/// The server configuration could not be loaded.
///
/// Boxes the underlying `figment::Error`, which is large, so this error type stays
/// small enough that `clippy::result_large_err` does not flag `ServerConfig::load`.
#[derive(Debug, Error)]
#[error("invalid server config: {0}")]
pub struct ConfigError(Box<figment::Error>);

impl ServerConfig {
    /// Loads `config/server.toml`, then `RRM_`-prefixed environment variables, then a
    /// bare `PORT`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] if any layer fails to parse or a required field is missing.
    pub fn load() -> Result<Self, ConfigError> {
        Figment::new()
            .merge(Toml::file("config/server.toml"))
            .merge(Env::prefixed("RRM_"))
            .merge(Env::raw().only(&["PORT"]).map(|_| "port".into()))
            .extract()
            .map_err(|err| ConfigError(Box::new(err)))
    }
}
