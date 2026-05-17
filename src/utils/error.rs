use std::fmt::Display;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Extension trait for converting `Result<T, E>` to `Result<T, Response>`
/// with appropriate status codes and logging.
#[allow(clippy::result_large_err)]
pub trait OrStatus<T> {
    /// Log at `error` level and return `500 Internal Server Error`.
    fn or_500(self, msg: &str) -> Result<T, Response>;
    /// Log at `debug` level and return `404 Not Found`.
    fn or_404(self, msg: &str) -> Result<T, Response>;
    /// Log at `debug` level and return `400 Bad Request`.
    fn or_400(self, msg: &str) -> Result<T, Response>;
    /// Log at `error` level and return a custom status code.
    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, Response>;
}

impl<T, E: Display> OrStatus<T> for Result<T, E> {
    fn or_500(self, msg: &str) -> Result<T, Response> {
        self.map_err(|e| {
            tracing::error!(error = %e, "{msg}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
    }

    fn or_404(self, msg: &str) -> Result<T, Response> {
        self.map_err(|e| {
            tracing::debug!(error = %e, "{msg}");
            StatusCode::NOT_FOUND.into_response()
        })
    }

    fn or_400(self, msg: &str) -> Result<T, Response> {
        self.map_err(|e| {
            tracing::debug!(error = %e, "{msg}");
            StatusCode::BAD_REQUEST.into_response()
        })
    }

    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, Response> {
        self.map_err(|e| {
            tracing::error!(error = %e, "{msg}");
            status.into_response()
        })
    }
}

/// Unwrap `Result<T, Response>` or return the `Err` from the enclosing function.
#[macro_export]
macro_rules! ok_or_return {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(resp) => return resp,
        }
    };
}
