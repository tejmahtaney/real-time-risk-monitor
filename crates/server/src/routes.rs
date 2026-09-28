//! Route definitions and handlers.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use engine::{
    AlertEvent, Concentration, EngineCommand, EngineReply, LimitStatus, PnlReport, PortfolioTotals,
    PositionId, PositionRisk, PositionSpec, UnderlyingRisk,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::error::ApiError;
use crate::state::AppState;

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

    Router::new()
        .route("/api/health", get(health))
        .route("/api/positions", get(list_positions).post(add_position))
        .route("/api/positions/{id}", delete(remove_position))
        .route("/api/greeks", get(greeks))
        .route("/api/risk-summary", get(risk_summary))
        .route("/api/pnl", get(pnl))
        .route("/api/alerts", get(alerts))
        .route("/api/control", post(control))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
}

/// `GET /api/health` response body.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    sim_time: DateTime<Utc>,
    last_tick_age_ms: i64,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let snapshot = state.engine.latest();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        sim_time: snapshot.sim_time,
        last_tick_age_ms: (Utc::now() - snapshot.wall_time).num_milliseconds(),
    })
}

async fn list_positions(State(state): State<AppState>) -> Json<Vec<PositionRisk>> {
    Json(state.engine.latest().positions.clone())
}

/// `GET /api/greeks` response body.
#[derive(Serialize)]
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
#[derive(Serialize)]
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

/// `GET /api/alerts` response body.
#[derive(Serialize)]
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
        EngineReply::Ack | EngineReply::Removed(_) => Err(ApiError::EngineUnavailable(
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
        EngineReply::Ack | EngineReply::Added(_) => Err(ApiError::EngineUnavailable(
            "unexpected reply from the engine".to_owned(),
        )),
    }
}

/// The JSON body `POST /api/control` accepts.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
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
