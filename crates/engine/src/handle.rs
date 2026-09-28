//! The handle a caller (the server) uses to talk to a spawned engine.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use risk::AlertEvent;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::command::{EngineCommand, EngineCommandError, EngineReply, EngineRequest};
use crate::error::EngineError;
use crate::snapshot::RiskSnapshot;

/// A handle to a running engine task. Cloning shares the same channels: every clone
/// talks to the same engine.
#[derive(Debug, Clone)]
pub struct EngineHandle {
    snapshots: watch::Receiver<Arc<RiskSnapshot>>,
    alerts: broadcast::Sender<AlertEvent>,
    requests: mpsc::Sender<EngineRequest>,
    dropped_records: Arc<AtomicU64>,
}

impl EngineHandle {
    /// Builds a handle over the given channels. Only [`crate::Engine::spawn`] calls
    /// this; it is not part of the public API of this crate.
    pub(crate) fn new(
        snapshots: watch::Receiver<Arc<RiskSnapshot>>,
        alerts: broadcast::Sender<AlertEvent>,
        requests: mpsc::Sender<EngineRequest>,
        dropped_records: Arc<AtomicU64>,
    ) -> Self {
        Self {
            snapshots,
            alerts,
            requests,
            dropped_records,
        }
    }

    /// Records dropped because the session recorder's channel was full, cumulative for
    /// the life of this engine. Surfaced on `/api/health` and in the session's final
    /// `meta.json`.
    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The latest published snapshot. Never blocks; always returns immediately.
    pub fn latest(&self) -> Arc<RiskSnapshot> {
        self.snapshots.borrow().clone()
    }

    /// Subscribes to alert state-change events as they are published.
    pub fn subscribe_alerts(&self) -> broadcast::Receiver<AlertEvent> {
        self.alerts.subscribe()
    }

    /// A fresh clone of the snapshot `watch::Receiver`, for a caller (a WebSocket
    /// client task) that wants to await `changed()` independently of every other
    /// subscriber. A `watch` channel holds exactly one value, so a receiver that falls
    /// behind never queues: it just sees the latest value whenever it next looks. See
    /// `server::ws` for the streaming layer this exists for.
    pub fn subscribe_snapshots(&self) -> watch::Receiver<Arc<RiskSnapshot>> {
        self.snapshots.clone()
    }

    /// Sends a command and awaits its reply.
    ///
    /// The outer `Result` is [`EngineError::Gone`] when the engine task has stopped and
    /// the channel closed before a reply arrived. The inner `Result` is the command's
    /// own outcome: `Ok` once applied, or a typed [`EngineCommandError`] when the engine
    /// is running fine but the request itself was invalid.
    ///
    /// # Errors
    ///
    /// [`EngineError::Gone`] if the engine task is no longer running.
    pub async fn send(
        &self,
        command: EngineCommand,
    ) -> Result<Result<EngineReply, EngineCommandError>, EngineError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.requests
            .send(EngineRequest {
                command,
                reply: reply_tx,
            })
            .await
            .map_err(|_| EngineError::Gone)?;
        reply_rx.await.map_err(|_| EngineError::Gone)
    }
}
