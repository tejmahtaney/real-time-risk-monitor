//! Engine error types.

use marketdata::CalibrationError;
use portfolio::PortfolioError;
use thiserror::Error;

/// Everything that can go wrong building or talking to an [`crate::Engine`].
///
/// [`EngineError::Calibration`] and [`EngineError::Portfolio`] only happen inside
/// [`crate::Engine::new`], while building the initial market state and resolving the
/// starting portfolio. [`EngineError::Gone`] only happens on [`crate::EngineHandle::send`],
/// when the engine task has stopped running. [`crate::Engine::tick`] never constructs
/// any of these: a feed or market-data failure there is handled by marking the
/// snapshot's `feed_status` degraded instead of returning `Err` (see its doc comment).
#[derive(Debug, Error, PartialEq)]
pub enum EngineError {
    /// The command channel closed while a request was in flight: the engine task has
    /// stopped running.
    #[error("the engine task is no longer running")]
    Gone,
    /// The initial market state could not be built from the calibrated seed.
    #[error(transparent)]
    Calibration(#[from] CalibrationError),
    /// The starting portfolio could not be resolved or added to the book.
    #[error(transparent)]
    Portfolio(#[from] PortfolioError),
}
