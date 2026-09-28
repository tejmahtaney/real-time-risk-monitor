//! The handle a caller (the server) uses to talk to a spawned engine.

use std::sync::Arc;

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
}

impl EngineHandle {
    /// Builds a handle over the given channels. Only [`crate::Engine::spawn`] calls
    /// this; it is not part of the public API of this crate.
    pub(crate) fn new(
        snapshots: watch::Receiver<Arc<RiskSnapshot>>,
        alerts: broadcast::Sender<AlertEvent>,
        requests: mpsc::Sender<EngineRequest>,
    ) -> Self {
        Self {
            snapshots,
            alerts,
            requests,
        }
    }

    /// The latest published snapshot. Never blocks; always returns immediately.
    pub fn latest(&self) -> Arc<RiskSnapshot> {
        self.snapshots.borrow().clone()
    }

    /// Subscribes to alert state-change events as they are published.
    pub fn subscribe_alerts(&self) -> broadcast::Receiver<AlertEvent> {
        self.alerts.subscribe()
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
