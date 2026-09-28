//! Streams live snapshots and alert events to WebSocket clients.
//!
//! # Why `watch`, not a queue, is the whole design
//!
//! Each client task holds its own `watch::Receiver<Arc<RiskSnapshot>>`, cloned from
//! the engine's one `watch::Sender` via `EngineHandle::subscribe_snapshots`. A `watch`
//! channel holds exactly one value, the newest one published; sending a new value
//! overwrites whatever was there, it never queues. `Receiver::changed()` marks the
//! current value "seen" the moment it returns, so if a client's task is slow (a
//! stalled TCP write, a busy event loop, a burst of ticks) it does not build up a
//! backlog: on its next poll it simply sees "yes, something changed" once and reads
//! whatever the latest value is, skipping every intermediate tick it missed. The
//! engine's publish side (`snapshot_tx.send(..)`, inside the engine's own tick loop)
//! never blocks and never has to know or care how many clients exist or how fast they
//! are — there is no per-client queue for the engine to feed, so a slow or dead client
//! cannot slow the engine down or grow unbounded memory. This is the single most
//! important property of this module: back-pressure is structural, not something
//! implemented here with a bounded channel and a drop policy.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use engine::{AlertEvent, RiskSnapshot};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant as TokioInstant;

use crate::state::AppState;

/// Bumped only on a breaking change to this wire protocol.
const PROTOCOL_VERSION: u32 = 1;
/// How often a ping is sent to each client.
const PING_INTERVAL: Duration = Duration::from_secs(15);
/// A client is dropped if no pong arrives within this long.
const PONG_TIMEOUT: Duration = Duration::from_secs(30);
/// The fastest rate a client may throttle itself to, regardless of what it asks for.
const MAX_HZ: f64 = 10.0;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// One message this server sends over `/ws`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    /// Sent first, once, right after the socket upgrades.
    Hello {
        /// This wire protocol's version.
        protocol_version: u32,
        /// This build's `CARGO_PKG_VERSION`.
        server_version: &'static str,
        /// Real milliseconds between engine ticks.
        tick_interval_ms: u64,
        /// The clock's speed multiplier at connect time.
        speed: f64,
    },
    /// The full engine snapshot. Sent right after `Hello`, then on every change,
    /// subject to the client's throttle.
    Snapshot {
        /// The snapshot itself.
        snapshot: Arc<RiskSnapshot>,
    },
    /// One limit's state-change event, as it happens.
    Alert {
        /// The event.
        alert: AlertEvent,
    },
    /// Tells the client it missed one or more alert events; a fresh `snapshot`
    /// follows immediately, which is authoritative.
    Resync {
        /// Why this was sent.
        reason: String,
    },
    /// A problem with something the client sent.
    Error {
        /// What was wrong.
        message: String,
    },
}

/// One message a client may send over `/ws`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// Caps how often `snapshot` messages are sent to this connection.
    Subscribe {
        /// The requested rate, clamped to at most [`MAX_HZ`].
        max_hz: f64,
    },
}

/// `GET /ws`: upgrades the connection and hands it to [`run`].
pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

/// Logs connect/disconnect and keeps [`AppState::ws_clients`] accurate across every
/// exit path from [`run`] (a `break`, an early `return`, or the task simply ending
/// when the server shuts down).
struct ConnectionGuard {
    id: u64,
    clients: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    fn new(id: u64, clients: Arc<AtomicUsize>) -> Self {
        clients.fetch_add(1, Ordering::Relaxed);
        tracing::info!(client_id = id, "websocket client connected");
        Self { id, clients }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.clients.fetch_sub(1, Ordering::Relaxed);
        tracing::info!(client_id = self.id, "websocket client disconnected");
    }
}

/// Drives one client's connection until it closes, the engine goes away, or the
/// server shuts down. Axum's own graceful shutdown drains in-flight connections, so
/// no extra `CancellationToken` needs to be threaded in here.
async fn run(mut socket: WebSocket, state: AppState) {
    let id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnectionGuard::new(id, Arc::clone(&state.ws_clients));

    let mut snapshot_rx = state.engine.subscribe_snapshots();
    let mut alert_rx = state.engine.subscribe_alerts();

    let hello = ServerMessage::Hello {
        protocol_version: PROTOCOL_VERSION,
        server_version: env!("CARGO_PKG_VERSION"),
        tick_interval_ms: state.tick_interval_ms,
        speed: snapshot_rx.borrow().clock_speed,
    };
    if !send_json(&mut socket, &hello).await {
        return;
    }
    let initial = Arc::clone(&snapshot_rx.borrow_and_update());
    if !send_json(&mut socket, &ServerMessage::Snapshot { snapshot: initial }).await {
        return;
    }

    let mut throttle = Throttle::new();
    let mut last_pong = Instant::now();
    let mut ping_interval =
        tokio::time::interval_at(TokioInstant::now() + PING_INTERVAL, PING_INTERVAL);
    let trailing_sleep = tokio::time::sleep(Duration::from_secs(0));
    tokio::pin!(trailing_sleep);

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > PONG_TIMEOUT {
                    tracing::info!(client_id = id, "no pong within 30s, closing");
                    break;
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            changed = snapshot_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = Arc::clone(&snapshot_rx.borrow_and_update());
                match throttle.on_change(Instant::now()) {
                    Decision::Send => {
                        if !send_json(&mut socket, &ServerMessage::Snapshot { snapshot }).await {
                            break;
                        }
                    }
                    Decision::Defer(deadline) => {
                        trailing_sleep.as_mut().reset(TokioInstant::from_std(deadline));
                    }
                    Decision::Skip => {}
                }
            }
            () = &mut trailing_sleep, if throttle.pending() => {
                if let Decision::Send = throttle.on_timer(Instant::now()) {
                    let snapshot = Arc::clone(&snapshot_rx.borrow());
                    if !send_json(&mut socket, &ServerMessage::Snapshot { snapshot }).await {
                        break;
                    }
                }
            }
            alert = alert_rx.recv() => {
                if !handle_alert(&mut socket, alert, id, &mut snapshot_rx).await {
                    break;
                }
            }
            incoming = socket.recv() => {
                if !handle_incoming(&mut socket, incoming, id, &mut throttle, &mut last_pong).await {
                    break;
                }
            }
        }
    }
}

/// Handles one alert-broadcast event. Returns `false` when the connection should
/// close.
async fn handle_alert(
    socket: &mut WebSocket,
    alert: Result<AlertEvent, broadcast::error::RecvError>,
    id: u64,
    snapshot_rx: &mut watch::Receiver<Arc<RiskSnapshot>>,
) -> bool {
    match alert {
        Ok(event) => send_json(socket, &ServerMessage::Alert { alert: event }).await,
        Err(broadcast::error::RecvError::Lagged(missed)) => {
            tracing::warn!(
                client_id = id,
                missed,
                "alert broadcast lagged; resyncing client"
            );
            let reason = format!("missed {missed} alert events");
            if !send_json(socket, &ServerMessage::Resync { reason }).await {
                return false;
            }
            let snapshot = Arc::clone(&snapshot_rx.borrow_and_update());
            send_json(socket, &ServerMessage::Snapshot { snapshot }).await
        }
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

/// Handles one incoming client frame. Returns `false` when the connection should
/// close.
async fn handle_incoming(
    socket: &mut WebSocket,
    incoming: Option<Result<Message, axum::Error>>,
    id: u64,
    throttle: &mut Throttle,
    last_pong: &mut Instant,
) -> bool {
    match incoming {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Subscribe { max_hz }) => match throttle.set_max_hz(max_hz) {
                Ok(()) => true,
                Err(err) => {
                    let message = ServerMessage::Error {
                        message: err.to_string(),
                    };
                    send_json(socket, &message).await
                }
            },
            Err(err) => {
                let message = ServerMessage::Error {
                    message: format!("bad client message: {err}"),
                };
                send_json(socket, &message).await
            }
        },
        Some(Ok(Message::Pong(_))) => {
            *last_pong = Instant::now();
            true
        }
        // Tungstenite auto-answers a client's own `Ping` with a `Pong` beneath us;
        // binary frames are unused by this protocol.
        Some(Ok(Message::Ping(_) | Message::Binary(_))) => true,
        Some(Ok(Message::Close(_))) | None => false,
        Some(Err(err)) => {
            tracing::warn!(client_id = id, error = %err, "websocket read error");
            false
        }
    }
}

/// Serialises `message` and sends it as a text frame. Returns `false` when the socket
/// itself should be treated as closed; a serialisation failure (practically
/// unreachable for these types) falls back to a literal error message instead of
/// panicking, since `unwrap`/`expect` are banned outside tests.
async fn send_json(socket: &mut WebSocket, message: &ServerMessage) -> bool {
    let text = serde_json::to_string(message).unwrap_or_else(|err| {
        tracing::error!(error = %err, "failed to serialise a websocket message");
        r#"{"type":"error","message":"serialisation failed"}"#.to_owned()
    });
    socket.send(Message::text(text)).await.is_ok()
}

/// A pure, synchronously-testable throttle decision for the snapshot stream. Kept free
/// of any I/O, `tokio` time types or the socket itself, so its trailing-flush
/// behaviour (see [`Decision::Defer`]) can be exercised with synthetic [`Instant`]s
/// instead of real sleeps or a real engine — see this module's tests.
#[derive(Debug)]
struct Throttle {
    /// `None` means unthrottled: send on every change, whatever the natural tick rate is.
    min_interval: Option<Duration>,
    /// When the last snapshot was actually sent (send, not defer).
    last_sent: Option<Instant>,
    /// Whether a change was skipped and is owed a trailing flush.
    pending: bool,
}

/// What a [`Throttle`] decided to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// Send the snapshot now.
    Send,
    /// Hold the snapshot; a trailing flush should fire at this deadline if nothing
    /// else sends before then.
    Defer(Instant),
    /// Nothing is owed (the timer fired with nothing pending).
    Skip,
}

/// [`Throttle::set_max_hz`] was given something that isn't a usable rate.
#[derive(Debug, Clone, Copy, Error)]
#[error("max_hz must be a positive, finite number, got {0}")]
struct ThrottleError(f64);

impl Throttle {
    fn new() -> Self {
        Self {
            min_interval: None,
            last_sent: None,
            pending: false,
        }
    }

    /// Validates and applies a client's `subscribe` request. Rejects non-finite or
    /// non-positive `max_hz`; caps the effective rate at [`MAX_HZ`] regardless of what
    /// was requested.
    fn set_max_hz(&mut self, max_hz: f64) -> Result<(), ThrottleError> {
        if !max_hz.is_finite() || max_hz <= 0.0 {
            return Err(ThrottleError(max_hz));
        }
        let effective = max_hz.min(MAX_HZ);
        self.min_interval = Some(Duration::from_secs_f64(1.0 / effective));
        Ok(())
    }

    /// Called when the underlying snapshot changed at `now`.
    fn on_change(&mut self, now: Instant) -> Decision {
        let due = match (self.min_interval, self.last_sent) {
            (None, _) | (Some(_), None) => true,
            (Some(min), Some(last)) => now.duration_since(last) >= min,
        };
        if due {
            self.last_sent = Some(now);
            self.pending = false;
            Decision::Send
        } else {
            self.pending = true;
            let last = self.last_sent.unwrap_or(now);
            let min = self.min_interval.unwrap_or(Duration::ZERO);
            Decision::Defer(last + min)
        }
    }

    /// Called when the trailing timer fires at `now`.
    fn on_timer(&mut self, now: Instant) -> Decision {
        if self.pending {
            self.last_sent = Some(now);
            self.pending = false;
            Decision::Send
        } else {
            Decision::Skip
        }
    }

    /// Whether a trailing flush is currently owed.
    fn pending(&self) -> bool {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{NaiveDate, TimeZone, Utc};
    use engine::{FeedStatus, LimitKind, LimitState, PnlLevel, PnlReport, RiskSummary, Scope};
    use serde_json::json;

    use super::*;

    #[test]
    fn unthrottled_always_sends() {
        let mut throttle = Throttle::new();
        let t0 = Instant::now();
        assert_eq!(throttle.on_change(t0), Decision::Send);
        assert_eq!(
            throttle.on_change(t0 + Duration::from_millis(1)),
            Decision::Send
        );
    }

    #[test]
    fn throttled_defers_then_flushes_via_the_trailing_timer() {
        let mut throttle = Throttle::new();
        throttle.set_max_hz(2.0).unwrap();
        let t0 = Instant::now();
        assert_eq!(throttle.on_change(t0), Decision::Send);

        let deadline = t0 + Duration::from_millis(500);
        assert_eq!(
            throttle.on_change(t0 + Duration::from_millis(100)),
            Decision::Defer(deadline)
        );
        assert!(throttle.pending());

        // A further change before the deadline stays deferred to the same deadline.
        assert_eq!(
            throttle.on_change(t0 + Duration::from_millis(200)),
            Decision::Defer(deadline)
        );

        assert_eq!(throttle.on_timer(deadline), Decision::Send);
        assert!(!throttle.pending());

        // Nothing pending: firing again does nothing.
        assert_eq!(
            throttle.on_timer(deadline + Duration::from_millis(1)),
            Decision::Skip
        );
    }

    #[test]
    fn max_hz_is_capped_at_ten() {
        let mut throttle = Throttle::new();
        throttle.set_max_hz(1_000.0).unwrap();
        assert_eq!(throttle.min_interval, Some(Duration::from_secs_f64(0.1)));
    }

    #[test]
    fn invalid_max_hz_is_rejected() {
        let mut throttle = Throttle::new();
        assert!(throttle.set_max_hz(0.0).is_err());
        assert!(throttle.set_max_hz(-1.0).is_err());
        assert!(throttle.set_max_hz(f64::NAN).is_err());
        assert!(throttle.set_max_hz(f64::INFINITY).is_err());
    }

    #[test]
    fn hello_message_shape() {
        let msg = ServerMessage::Hello {
            protocol_version: 1,
            server_version: "0.1.0",
            tick_interval_ms: 250,
            speed: 3_600.0,
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            value,
            json!({
                "type": "hello",
                "protocol_version": 1,
                "server_version": "0.1.0",
                "tick_interval_ms": 250,
                "speed": 3600.0,
            })
        );
    }

    fn sample_snapshot() -> RiskSnapshot {
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        RiskSnapshot {
            seq: 42,
            wall_time: Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap(),
            sim_time: Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap(),
            session_date: date,
            step_kind: engine::StepKind::Intraday,
            market: BTreeMap::new(),
            positions: Vec::new(),
            summary: RiskSummary::default(),
            pnl: PnlReport {
                session_date: date,
                portfolio: PnlLevel::default(),
                by_underlying: BTreeMap::new(),
                by_position: BTreeMap::new(),
                previous_day: None,
            },
            active_alerts: Vec::new(),
            limits: Vec::new(),
            alert_history: Vec::new(),
            clock_speed: 1.0,
            paused: false,
            feed_status: FeedStatus::Ok,
            compute_micros: 123,
        }
    }

    #[test]
    fn snapshot_message_envelope_shape() {
        let msg = ServerMessage::Snapshot {
            snapshot: Arc::new(sample_snapshot()),
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["type"], "snapshot");
        assert!(value["snapshot"].is_object());
        assert_eq!(value["snapshot"]["seq"], 42);
        assert_eq!(value["snapshot"]["compute_micros"], 123);
    }

    #[test]
    fn alert_message_shape() {
        let event = AlertEvent {
            id: 7,
            wall_time: Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap(),
            sim_time: Utc.with_ymd_and_hms(2026, 9, 28, 14, 30, 0).unwrap(),
            name: LimitKind::PortfolioDollarDelta,
            scope: Scope::Portfolio,
            from: LimitState::Ok,
            to: LimitState::Warning,
            value: 85_000.0,
            limit: 100_000.0,
        };
        let msg = ServerMessage::Alert { alert: event };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["type"], "alert");
        assert_eq!(value["alert"]["id"], 7);
        assert_eq!(value["alert"]["name"], "portfolio_dollar_delta");
        assert_eq!(value["alert"]["scope"], json!({"kind": "portfolio"}));
        assert_eq!(value["alert"]["from"], "ok");
        assert_eq!(value["alert"]["to"], "warning");
        assert_eq!(value["alert"]["value"], 85_000.0);
        assert_eq!(value["alert"]["limit"], 100_000.0);
    }

    #[test]
    fn resync_message_shape() {
        let msg = ServerMessage::Resync {
            reason: "missed 3 alert events".to_owned(),
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            value,
            json!({"type": "resync", "reason": "missed 3 alert events"})
        );
    }

    #[test]
    fn error_message_shape() {
        let msg = ServerMessage::Error {
            message: "bad input".to_owned(),
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value, json!({"type": "error", "message": "bad input"}));
    }

    #[test]
    fn subscribe_message_parses() {
        let value = json!({"type": "subscribe", "max_hz": 2.0});
        let msg: ClientMessage = serde_json::from_value(value).unwrap();
        let ClientMessage::Subscribe { max_hz } = msg;
        assert!((max_hz - 2.0).abs() < f64::EPSILON);
    }
}
