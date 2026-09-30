//! The axum REST layer and config loading. Exposed as a library so integration tests
//! can build the router.

#![warn(missing_docs)]

mod backtest_routes;
mod config;
mod error;
mod routes;
mod state;
mod storage_config;
mod storage_routes;
mod validate;
mod ws;

pub use config::{ConfigError, ServerConfig};
pub use error::ApiError;
pub use state::AppState;
pub use storage_config::{ConfigError as StorageConfigError, StartPortfolio, StorageConfig};

use axum::Router;

/// Builds the full router: every route under `/api`, plus tracing, CORS, a request
/// timeout and a body size limit, over the given [`AppState`].
pub fn build_router(state: AppState) -> Router {
    routes::router(state)
}
