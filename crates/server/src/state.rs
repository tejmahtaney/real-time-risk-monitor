//! Shared application state.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use engine::EngineHandle;

use crate::config::ServerConfig;

/// State shared across every axum handler.
#[derive(Clone)]
pub struct AppState {
    /// Handle to the running engine.
    pub engine: EngineHandle,
    /// Resolved server configuration.
    pub config: Arc<ServerConfig>,
    /// When the server process started, for `/api/health`'s uptime.
    pub started_at: Instant,
    /// Real milliseconds between engine ticks, from `config/simulation.toml`. Sent in
    /// the WebSocket `hello` message. Not part of `ServerConfig` because it comes from
    /// a separately loaded config (`marketdata::SimulationConfig`).
    pub tick_interval_ms: u64,
    /// Count of currently connected WebSocket clients, surfaced on `/api/health`.
    pub ws_clients: Arc<AtomicUsize>,
}
