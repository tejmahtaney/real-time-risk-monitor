//! The backtester's REST and WebSocket endpoints (Part 20). A thin layer over
//! [`engine::BacktestManager`]: parse the request, call the manager, map its errors.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{delete, get, post};
use engine::{
    BacktestListItem, BacktestResults, BacktestStatus, BacktestSubscription, ControlCommand,
    CreateOptions, DEFAULT_SPEED, RunInput,
};
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;
use crate::validate::safe_segment;
use crate::ws::{self, StreamSource};

/// The largest price file accepted, in bytes: 5 MB.
pub const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

/// Every backtest route except the upload. Merged under the router's small (64 KB) body limit.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/backtests", get(list))
        .route("/api/backtests/{id}", get(status).delete(stop))
        .route("/api/backtests/{id}/control", post(control))
        .route("/api/backtests/{id}/results", get(results))
        .route("/api/backtests/{id}/files", delete(delete_files))
        .route("/ws/backtests/{id}", get(ws_handler))
}

/// `POST /api/backtests`, on its own so it can carry a 5 MB body limit (`DefaultBodyLimit`) while
/// every other route keeps the small one.
pub fn upload_router() -> Router<AppState> {
    Router::new()
        .route("/api/backtests", post(create))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}

/// The JSON body of `POST /api/backtests` that replays the local archive instead of an upload.
#[derive(Deserialize, ts_rs::TS)]
#[serde(tag = "source", rename_all = "snake_case")]
#[ts(export)]
enum BacktestCreateRequest {
    /// Every archived download, merged per symbol.
    Archive {
        /// Replace an expired option with a new one from the same portfolio row. Default true.
        #[serde(default)]
        #[ts(optional)]
        roll_expired: Option<bool>,
        /// Dates per second when playing, 1 to 50. Default 5.
        #[serde(default)]
        #[ts(optional)]
        speed: Option<f64>,
    },
}

/// The JSON body of `POST /api/backtests/{id}/control`.
#[derive(Deserialize, ts_rs::TS)]
#[serde(tag = "action", rename_all = "snake_case")]
#[ts(export)]
enum BacktestControlRequest {
    /// Step at the run's speed.
    Play,
    /// Stop stepping.
    Pause,
    /// Replay one date, then stay paused.
    Step,
    /// Change the speed, in dates per second from 1 to 50.
    SetSpeed { speed: f64 },
    /// Compute every remaining date without delay.
    RunToEnd,
}

/// The JSON body of `DELETE /api/backtests/{id}/files`: the run's own id, typed out.
#[derive(Deserialize, ts_rs::TS)]
#[ts(export)]
struct BacktestDeleteFilesRequest {
    /// Must equal the id in the URL.
    confirm: String,
}

async fn create(
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, Json<BacktestStatus>), ApiError> {
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|len| len > MAX_UPLOAD_BYTES) {
        return Err(too_large());
    }
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let (input, options) = if content_type.starts_with("multipart/form-data") {
        read_upload(request, &state).await?
    } else if content_type.starts_with("application/json") {
        let body = Json::<BacktestCreateRequest>::from_request(request, &state)
            .await
            .map_err(|e| json_rejection(&e))?
            .0;
        let BacktestCreateRequest::Archive {
            roll_expired,
            speed,
        } = body;
        (
            RunInput::Archive,
            CreateOptions {
                roll_expired: roll_expired.unwrap_or(true),
                speed: speed.unwrap_or(DEFAULT_SPEED),
            },
        )
    } else {
        return Err(ApiError::Validation(
            "send a multipart/form-data upload with a `file` field, or JSON {\"source\":\"archive\"}"
                .to_owned(),
        ));
    };
    let status = state.backtests.create(input, options).await?;
    Ok((StatusCode::CREATED, Json(status)))
}

/// Reads a multipart upload: a `file` field with the CSV, and optional `roll_expired` and `speed`.
async fn read_upload(
    request: Request,
    state: &AppState,
) -> Result<(RunInput, CreateOptions), ApiError> {
    let mut multipart = Multipart::from_request(request, state)
        .await
        .map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                too_large()
            } else {
                ApiError::Validation(rejection.body_text())
            }
        })?;
    let mut file: Option<(Option<String>, Vec<u8>)> = None;
    let mut options = CreateOptions::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| multipart_error(&e))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "file" => {
                let filename = field.file_name().map(str::to_owned);
                let bytes = field.bytes().await.map_err(|e| multipart_error(&e))?;
                file = Some((filename, bytes.to_vec()));
            }
            "roll_expired" => {
                let text = field.text().await.map_err(|e| multipart_error(&e))?;
                options.roll_expired = text.trim().parse().map_err(|_| {
                    ApiError::Validation("roll_expired must be true or false".to_owned())
                })?;
            }
            "speed" => {
                let text = field.text().await.map_err(|e| multipart_error(&e))?;
                options.speed = text.trim().parse().map_err(|_| {
                    ApiError::Validation("speed must be a number of dates per second".to_owned())
                })?;
            }
            _ => {}
        }
    }
    let (filename, bytes) = file.ok_or_else(|| {
        ApiError::Validation("the upload has no `file` field holding the price CSV".to_owned())
    })?;
    Ok((RunInput::Upload { filename, bytes }, options))
}

fn too_large() -> ApiError {
    ApiError::PayloadTooLarge(format!(
        "the price file is over the {} MB limit",
        MAX_UPLOAD_BYTES / (1024 * 1024)
    ))
}

fn multipart_error(error: &axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        too_large()
    } else {
        ApiError::Validation(error.body_text())
    }
}

fn json_rejection(rejection: &JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        too_large()
    } else {
        ApiError::Validation(rejection.body_text())
    }
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<BacktestListItem>>, ApiError> {
    Ok(Json(state.backtests.list()?))
}

async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BacktestStatus>, ApiError> {
    safe_segment(&id)?;
    Ok(Json(state.backtests.status(&id)?))
}

async fn control(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<BacktestControlRequest>,
) -> Result<Json<BacktestStatus>, ApiError> {
    safe_segment(&id)?;
    let command = match request {
        BacktestControlRequest::Play => ControlCommand::Play,
        BacktestControlRequest::Pause => ControlCommand::Pause,
        BacktestControlRequest::Step => ControlCommand::Step,
        BacktestControlRequest::SetSpeed { speed } => ControlCommand::SetSpeed(speed),
        BacktestControlRequest::RunToEnd => ControlCommand::RunToEnd,
    };
    Ok(Json(state.backtests.control(&id, command)?))
}

async fn results(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BacktestResults>, ApiError> {
    safe_segment(&id)?;
    Ok(Json(state.backtests.results(&id)?))
}

/// `DELETE /api/backtests/{id}`: stops the replay and frees its memory. The files stay.
async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    safe_segment(&id)?;
    state.backtests.stop(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/backtests/{id}/files`: deletes a saved run's folder. The body must be
/// `{"confirm":"<id>"}`. It is read and checked before anything else, so a request without it
/// changes nothing. This is the only delete in the app, and only a user's request reaches it.
async fn delete_files(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<BacktestDeleteFilesRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(body) = body.map_err(|_| {
        ApiError::Validation(
            "to delete a run's files, send a JSON body {\"confirm\": \"<the run id>\"}".to_owned(),
        )
    })?;
    if body.confirm != id {
        return Err(ApiError::Validation(
            "confirm must equal the run id in the URL, nothing was deleted".to_owned(),
        ));
    }
    safe_segment(&id)?;
    state.backtests.delete_files(&id, &body.confirm).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /ws/backtests/{id}`: streams a replay with the same protocol as `/ws`. An unknown run, or
/// one no longer in memory, gets a JSON 404 before the upgrade.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    safe_segment(&id)?;
    let subscription = state.backtests.subscribe(&id)?;
    let ws_clients = Arc::clone(&state.ws_clients);
    Ok(ws.on_upgrade(move |socket| async move {
        let BacktestSubscription {
            snapshots,
            alerts,
            speed,
            guard,
        } = subscription;
        // `tick_interval_ms` tells the client how often to expect a snapshot: one per date.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tick_interval_ms = (1_000.0 / speed.max(f64::MIN_POSITIVE)).round() as u64;
        let source = StreamSource {
            snapshot_rx: snapshots,
            alert_rx: alerts,
            tick_interval_ms,
            speed,
        };
        ws::stream(socket, ws_clients, source).await;
        // Held to here, so the run counts as in use for as long as the client is connected.
        drop(guard);
    }))
}
