//! The one error type every handler returns through, and its JSON shape.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use engine::{DomainError, EngineCommandError, EngineError, PortfolioError};
use serde::Serialize;
use thiserror::Error;

/// Every failure an API handler can produce. `IntoResponse` turns each into
/// `{"error":{"code":"...","message":"..."}}` with the matching HTTP status. No handler
/// ever returns a bare 500 string.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The request body or a path parameter failed validation. 400.
    #[error("{0}")]
    Validation(String),
    /// The named id does not exist. 404.
    #[error("{0}")]
    NotFound(String),
    /// The engine is not ready to answer: its channel closed. 503.
    #[error("{0}")]
    EngineUnavailable(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl ApiError {
    fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation_error",
            Self::NotFound(_) => "not_found",
            Self::EngineUnavailable(_) => "engine_unavailable",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::EngineUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message: self.to_string(),
            },
        };
        (self.status(), Json(body)).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(err: EngineError) -> Self {
        Self::EngineUnavailable(err.to_string())
    }
}

impl From<EngineCommandError> for ApiError {
    fn from(err: EngineCommandError) -> Self {
        match &err {
            EngineCommandError::InvalidSpeed(_) => Self::Validation(err.to_string()),
            EngineCommandError::Portfolio(portfolio_error) => match portfolio_error {
                PortfolioError::UnknownPosition(_) => Self::NotFound(err.to_string()),
                PortfolioError::DuplicateId(_)
                | PortfolioError::MissingUnderlying { .. }
                | PortfolioError::MissingField { .. }
                | PortfolioError::InvalidStrike { .. }
                | PortfolioError::InvalidExpiry { .. }
                | PortfolioError::Domain(_)
                | PortfolioError::Pricing(_)
                | PortfolioError::Csv { .. }
                | PortfolioError::Io { .. } => Self::Validation(err.to_string()),
            },
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self::Validation(err.to_string())
    }
}
