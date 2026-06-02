//! Error-to-status-code conversion — `OrStatus` trait and `IntoResolved` trait.

use std::fmt::Display;

use axum::http::StatusCode;

use crate::utils::path::ResolveTargetError;

/// Extension trait for converting `Result<T, E>` to `Result<T, StatusCode>`
/// with appropriate status codes and logging.
///
/// Log level convention:
/// - 4xx codes → `debug!` (client error, normal operation)
/// - 5xx codes → `error!` (server error, requires attention)
pub trait OrStatus<T> {
    /// Log the error and convert to `StatusCode`.
    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized;

    /// Log at `debug` level and return `400 Bad Request`.
    fn or_400(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::BAD_REQUEST, msg)
    }

    /// Log at `debug` level and return `403 Forbidden`.
    fn or_403(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::FORBIDDEN, msg)
    }

    /// Log at `debug` level and return `404 Not Found`.
    fn or_404(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::NOT_FOUND, msg)
    }

    /// Log at `debug` level and return `409 Conflict`.
    #[allow(dead_code)]
    fn or_409(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::CONFLICT, msg)
    }

    /// Log at `error` level and return `500 Internal Server Error`.
    fn or_500(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }

    /// Log at `error` level and return `503 Service Unavailable`.
    #[allow(dead_code)]
    fn or_503(self, msg: &str) -> Result<T, StatusCode>
    where
        Self: Sized,
    {
        self.or_status(StatusCode::SERVICE_UNAVAILABLE, msg)
    }
}

impl<T, E: Display> OrStatus<T> for Result<T, E> {
    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, StatusCode> {
        self.map_err(|e| {
            if status.is_server_error() {
                tracing::error!(error = %e, "{msg}");
            } else {
                tracing::debug!(error = %e, "{msg}");
            }
            status
        })
    }
}

impl<T> OrStatus<T> for Option<T> {
    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, StatusCode> {
        self.ok_or_else(|| {
            if status.is_server_error() {
                tracing::error!("{msg}");
            } else {
                tracing::debug!("{msg}");
            }
            status
        })
    }
}

/// Extension trait for converting `Result<T, ResolveTargetError>` into
/// `Result<T, StatusCode>` for handler use.
///
/// Wraps [`ResolveTargetError`](crate::utils::path::ResolveTargetError) by
/// logging the failure and converting to the appropriate HTTP status code
/// via [`ResolveTargetError::status`](crate::utils::path::ResolveTargetError::status).
pub trait IntoResolved<T> {
    fn or_invalid(self, on_invalid: StatusCode) -> Result<T, StatusCode>;
}

impl<T> IntoResolved<T> for Result<T, ResolveTargetError> {
    fn or_invalid(self, on_invalid: StatusCode) -> Result<T, StatusCode> {
        if let Err(e) = self.as_ref() {
            tracing::debug!(error = ?e, "path resolution failed");
        }
        self.map_err(|e| e.status(on_invalid))
    }
}
