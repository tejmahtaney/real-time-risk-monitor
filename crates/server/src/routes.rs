//! Route definitions and handlers.

use std::time::Duration;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use engine::{
    AlertEvent, Concentration, EngineCommand, EngineReply, HistoryPoint, LimitStatus, PnlReport,
    PortfolioTotals, PositionId, PositionRisk, PositionSpec, UnderlyingRisk, VarReport,
};
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::error::ApiError;
use crate::state::AppState;
use crate::{storage_routes, ws};

/// The maximum size of a request body this server accepts, in bytes. Every route here
/// takes a small JSON body at most.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Assembles every route under `/api`, plus tracing, CORS, a request timeout and a
/// body size limit.
pub fn router(state: AppState) -> Router {
    let origins: Vec<_> = state
        .config
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any);

    let static_dir = state.config.static_dir.clone();
    let index = static_dir.join("index.html");
    let static_service = ServiceBuilder::new()
        .layer(middleware::from_fn(static_cache_headers))
        .service(ServeDir::new(static_dir).fallback(ServeFile::new(index)));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/positions", get(list_positions).post(add_position))
        .route("/api/positions/{id}", delete(remove_position))
        .route("/api/greeks", get(greeks))
        .route("/api/risk-summary", get(risk_summary))
        .route("/api/pnl", get(pnl))
        .route("/api/var", get(var))
        .route("/api/alerts", get(alerts))
        .route("/api/control", post(control))
        .route("/api/history", get(history))
        .route("/ws", get(ws::handler))
        // Catches any other `/api/...` path so an unknown API route gets the usual
        // typed 404 instead of falling through to the SPA's `index.html` below —
        // matched only when nothing more specific above already matched, since axum's
        // router prefers literal segments over a wildcard regardless of registration
        // order.
        .route("/api/{*rest}", any(api_not_found))
        .merge(storage_routes::router())
        .with_state(state)
        // Everything that is not `/api/...` or `/ws` falls here: the built frontend,
        // with any unmatched client-side route (e.g. a deep link) falling back to
        // `index.html` so the SPA's own router can take over.
        .fallback_service(static_service)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound("no such API route".to_owned())
}

/// Sets `Cache-Control` on every static response: a year, immutable, for a file under
/// Vite's hashed `/assets/` output directory (its URL changes whenever its content
/// does); `no-cache` for everything else, in particular `index.html` itself and the
/// SPA fallback that serves it for an unmatched client-side route, since that file's
/// content can change on a new deploy without its URL changing at all.
async fn static_cache_headers(uri: Uri, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let value = if uri.path().starts_with("/assets/") {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    } else {
        HeaderValue::from_static("no-cache")
    };
    response.headers_mut().insert(header::CACHE_CONTROL, value);
    response
}

/// `GET /api/health` response body.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    sim_time: DateTime<Utc>,
    last_tick_age_ms: i64,
    connected_ws_clients: usize,
    /// Records dropped because the session recorder's channel was full, cumulative for
    /// this run.
    dropped_records: u64,
    /// Set once storage usage crosses `storage.toml`'s `warn_fraction`.
    storage_warning: bool,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let snapshot = state.engine.latest();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        sim_time: snapshot.sim_time,
        last_tick_age_ms: (Utc::now() - snapshot.wall_time).num_milliseconds(),
        connected_ws_clients: state.ws_clients.load(std::sync::atomic::Ordering::Relaxed),
        dropped_records: state.engine.dropped_records(),
        storage_warning: state
            .storage_warning
            .load(std::sync::atomic::Ordering::Relaxed),
    })
}

async fn list_positions(State(state): State<AppState>) -> Json<Vec<PositionRisk>> {
    Json(state.engine.latest().positions.clone())
}

/// `GET /api/greeks` response body.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct GreeksResponse {
    by_underlying: std::collections::BTreeMap<engine::Symbol, UnderlyingRisk>,
    totals: PortfolioTotals,
}

async fn greeks(State(state): State<AppState>) -> Json<GreeksResponse> {
    let snapshot = state.engine.latest();
    Json(GreeksResponse {
        by_underlying: snapshot.summary.by_underlying.clone(),
        totals: snapshot.summary.totals,
    })
}

/// `GET /api/risk-summary` response body.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct RiskSummaryResponse {
    totals: PortfolioTotals,
    concentration: Vec<Concentration>,
    limits: Vec<LimitStatus>,
    compute_micros: u64,
}

async fn risk_summary(State(state): State<AppState>) -> Json<RiskSummaryResponse> {
    let snapshot = state.engine.latest();
    Json(RiskSummaryResponse {
        totals: snapshot.summary.totals,
        concentration: snapshot.summary.concentration.clone(),
        limits: snapshot.active_alerts.clone(),
        compute_micros: snapshot.compute_micros,
    })
}

async fn pnl(State(state): State<AppState>) -> Json<PnlReport> {
    Json(state.engine.latest().pnl.clone())
}

/// `GET /api/var`: one-day Value at Risk and Expected Shortfall by all three methods
/// (Part 18), plus which one is the headline.
async fn var(State(state): State<AppState>) -> Json<VarReport> {
    Json(state.engine.latest().var)
}

/// `GET /api/alerts` response body.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
struct AlertsResponse {
    active: Vec<LimitStatus>,
    history: Vec<AlertEvent>,
}

async fn alerts(State(state): State<AppState>) -> Json<AlertsResponse> {
    let snapshot = state.engine.latest();
    Json(AlertsResponse {
        active: snapshot.active_alerts.clone(),
        history: snapshot.alert_history.clone(),
    })
}

async fn add_position(
    State(state): State<AppState>,
    Json(spec): Json<PositionSpec>,
) -> Result<Json<PositionRisk>, ApiError> {
    let reply = state
        .engine
        .send(EngineCommand::AddPosition(spec))
        .await??;
    match reply {
        EngineReply::Added(row) => Ok(Json(*row)),
        EngineReply::Ack
        | EngineReply::Removed(_)
        | EngineReply::History(_)
        | EngineReply::Specs(_)
        | EngineReply::Loaded(_) => Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        )),
    }
}

async fn remove_position(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = PositionId::new(&id)?;
    let reply = state
        .engine
        .send(EngineCommand::RemovePosition(id))
        .await??;
    match reply {
        EngineReply::Removed(_) => Ok(StatusCode::NO_CONTENT),
        EngineReply::Ack
        | EngineReply::Added(_)
        | EngineReply::History(_)
        | EngineReply::Specs(_)
        | EngineReply::Loaded(_) => Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        )),
    }
}

/// `GET /api/history` query parameters.
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    since: Option<u64>,
}

async fn history(
    State(state): State<AppState>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryPoint>>, ApiError> {
    let reply = state
        .engine
        .send(EngineCommand::GetHistory {
            since: params.since,
        })
        .await??;
    match reply {
        EngineReply::History(points) => Ok(Json(points)),
        EngineReply::Ack
        | EngineReply::Added(_)
        | EngineReply::Removed(_)
        | EngineReply::Specs(_)
        | EngineReply::Loaded(_) => Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        )),
    }
}

/// The JSON body `POST /api/control` accepts.
#[derive(Deserialize, ts_rs::TS)]
#[serde(tag = "action", rename_all = "snake_case")]
#[ts(export)]
enum ControlRequest {
    Pause,
    Resume,
    SetSpeed { speed: f64 },
}

async fn control(
    State(state): State<AppState>,
    Json(request): Json<ControlRequest>,
) -> Result<StatusCode, ApiError> {
    let command = match request {
        ControlRequest::Pause => EngineCommand::Pause,
        ControlRequest::Resume => EngineCommand::Resume,
        ControlRequest::SetSpeed { speed } => EngineCommand::SetSpeed(speed),
    };
    state.engine.send(command).await??;
    Ok(StatusCode::NO_CONTENT)
}
