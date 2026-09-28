//! The axum REST layer and config loading. Exposed as a library so integration tests
//! can build the router.

#![warn(missing_docs)]

mod config;
mod error;
mod routes;
mod state;

pub use config::{ConfigError, ServerConfig};
pub use error::ApiError;
pub use state::AppState;

use axum::Router;

/// Builds the full router: every route under `/api`, plus tracing, CORS, a request
/// timeout and a body size limit, over the given [`AppState`].
pub fn build_router(state: AppState) -> Router {
    routes::router(state)
}
