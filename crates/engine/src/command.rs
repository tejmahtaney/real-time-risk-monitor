//! Commands sent to a running engine, and their request/reply envelope.

use domain::{Position, PositionId};
use portfolio::{PortfolioError, PositionRisk, PositionSpec};
use thiserror::Error;
use tokio::sync::oneshot;

/// A command the engine task accepts over its `mpsc` channel.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// Stop advancing simulated time.
    Pause,
    /// Resume advancing simulated time.
    Resume,
    /// Change the clock's speed multiplier. The engine validates this is between 1.0 and
    /// 3,600.0 inclusive, stricter than `domain::SimClock::set_speed`, which only rejects
    /// non-finite or non-positive values.
    SetSpeed(f64),
    /// Resolve a new position against the current market and add it to the book.
    AddPosition(PositionSpec),
    /// Remove a position from the book by id.
    RemovePosition(PositionId),
}

/// Everything that can go wrong executing an [`EngineCommand`], returned through its
/// `oneshot` reply.
#[derive(Debug, Error)]
pub enum EngineCommandError {
    /// `SetSpeed` was given a value outside 1.0..=3600.0, or non-finite.
    #[error("speed must be finite and between 1 and 3600, got {0}")]
    InvalidSpeed(f64),
    /// `AddPosition` or `RemovePosition` failed at the portfolio layer: a duplicate id,
    /// an unknown id, a missing underlying, a bad strike or expiry, and so on.
    #[error(transparent)]
    Portfolio(#[from] PortfolioError),
}

/// What each command replies with, once the engine has applied it.
#[derive(Debug)]
pub enum EngineReply {
    /// Acknowledges `Pause`, `Resume` or `SetSpeed`.
    Ack,
    /// The resolved position `AddPosition` added to the book, valued as of the moment it
    /// was added: the same shape `GET /api/positions` would show for it.
    Added(Box<PositionRisk>),
    /// The position `RemovePosition` took out of the book.
    Removed(Box<Position>),
}

/// One request sent down the engine's `mpsc::Sender<EngineRequest>`: a command plus a
/// `oneshot` channel the engine replies on.
#[derive(Debug)]
pub struct EngineRequest {
    /// The command to run.
    pub command: EngineCommand,
    /// Where the reply goes.
    pub reply: oneshot::Sender<Result<EngineReply, EngineCommandError>>,
}
