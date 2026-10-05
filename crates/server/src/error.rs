//! The one error type every handler returns through, and its JSON shape.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use engine::{
    BacktestError, DomainError, EngineCommandError, EngineError, MarketDataError, PortfolioError,
    StorageError,
};
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
    /// A storage read or write failed for a reason other than a bad id (see
    /// `Validation`) or a missing file (see `NotFound`). 503.
    #[error("{0}")]
    Storage(String),
    /// The request body is bigger than this route accepts, such as a price file over 5 MB. 413.
    #[error("{0}")]
    PayloadTooLarge(String),
    /// A limit on concurrent work is reached, such as two backtests already running. 429.
    #[error("{0}")]
    TooManyRequests(String),
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
            Self::Storage(_) => "storage_error",
            Self::PayloadTooLarge(_) => "payload_too_large",
            Self::TooManyRequests(_) => "too_many_requests",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::EngineUnavailable(_) | Self::Storage(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
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
                | PortfolioError::Csv { .. } => Self::Validation(err.to_string()),
                PortfolioError::Io { .. } => {
                    Self::Validation(redacted(&err, "the portfolio file could not be read"))
                }
            },
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self::Validation(err.to_string())
    }
}

/// Logs the full error, which may hold a file system path, and returns a message that does
/// not. Only `/api/storage` may show the storage path to the user.
fn redacted(err: &dyn std::error::Error, public: &str) -> String {
    tracing::warn!(error = %err, "request failed; the detail is kept out of the response");
    public.to_owned()
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        match &err {
            StorageError::UnsafePath(_) => Self::Validation(err.to_string()),
            StorageError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                Self::NotFound(redacted(&err, "that item was not found in local storage"))
            }
            StorageError::Io { action, .. } => {
                let public = format!("local storage could not {action}");
                Self::Storage(redacted(&err, &public))
            }
            StorageError::Json { .. } => Self::Storage(redacted(
                &err,
                "a stored file could not be read as valid JSON",
            )),
            StorageError::RepoRootNotFound(_) => {
                Self::Storage(redacted(&err, "the storage location could not be resolved"))
            }
        }
    }
}

impl From<PortfolioError> for ApiError {
    fn from(err: PortfolioError) -> Self {
        // Reached only when a route parses a version file's CSV directly (outside any
        // `EngineCommand`, so `From<EngineCommandError>` above does not apply) — a bad
        // file on disk reads like a bad request.
        match err {
            PortfolioError::Io { .. } => {
                Self::Validation(redacted(&err, "the portfolio file could not be read"))
            }
            _ => Self::Validation(err.to_string()),
        }
    }
}

impl From<BacktestError> for ApiError {
    fn from(err: BacktestError) -> Self {
        match err {
            BacktestError::Archive(MarketDataError::Storage(storage))
            | BacktestError::Storage(storage) => storage.into(),
            BacktestError::Archive(MarketDataError::Fallback { .. }) => {
                Self::Validation(redacted(&err, "a price file could not be read"))
            }
            BacktestError::Portfolio(portfolio) => portfolio.into(),
            BacktestError::Invalid(_)
            | BacktestError::Data(_)
            | BacktestError::Actions(_)
            | BacktestError::Engine(_)
            | BacktestError::Archive(_) => Self::Validation(err.to_string()),
            BacktestError::NotFound(_) => Self::NotFound(err.to_string()),
            BacktestError::TooManyRunning { .. } => Self::TooManyRequests(err.to_string()),
            BacktestError::Internal(_) => Self::Storage(redacted(
                &err,
                "the back-test could not complete because of an internal error",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn storage_errors_do_not_leak_the_path() {
        let secret = PathBuf::from("/home/someone/secret-storage/sessions/x");
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let err = StorageError::Io {
                action: "read file",
                path: secret.clone(),
                source: std::io::Error::from(kind),
            };
            let api = ApiError::from(err);
            assert!(!api.to_string().contains("secret-storage"), "{api}");
        }
        let json = serde_json::from_str::<u8>("x").unwrap_err();
        let api = ApiError::from(StorageError::Json {
            path: secret.clone(),
            source: json,
        });
        assert!(!api.to_string().contains("secret-storage"), "{api}");
        let api = ApiError::from(StorageError::RepoRootNotFound(secret));
        assert!(!api.to_string().contains("secret-storage"), "{api}");
    }

    #[test]
    fn portfolio_io_errors_do_not_leak_the_path() {
        let api = ApiError::from(PortfolioError::Io {
            path: "/home/someone/secret-storage/p.csv".to_owned(),
            message: "denied".to_owned(),
        });
        assert!(!api.to_string().contains("secret-storage"), "{api}");
    }
}
