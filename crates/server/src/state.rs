//! Shared application state.

use std::sync::Arc;
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
}
