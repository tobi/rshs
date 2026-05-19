use std::fmt::Display;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Extension trait for converting `Result<T, E>` to `Result<T, Response>`
/// with appropriate status codes and logging.
///
/// Log level convention:
///   4xx codes → `debug!` (client error, normal operation)
///   5xx codes → `error!` (server error, requires attention)
#[allow(clippy::result_large_err)]
pub trait OrStatus<T> {
    fn or_400(self, msg: &str) -> Result<T, Response>;
    fn or_404(self, msg: &str) -> Result<T, Response>;
    fn or_409(self, msg: &str) -> Result<T, Response>;
    fn or_500(self, msg: &str) -> Result<T, Response>;
    fn or_503(self, msg: &str) -> Result<T, Response>;
    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, Response>;
}

impl<T, E: Display> OrStatus<T> for Result<T, E> {
    fn or_400(self, msg: &str) -> Result<T, Response> {
        self.or_status(StatusCode::BAD_REQUEST, msg)
    }

    fn or_404(self, msg: &str) -> Result<T, Response> {
        self.or_status(StatusCode::NOT_FOUND, msg)
    }

    fn or_409(self, msg: &str) -> Result<T, Response> {
        self.or_status(StatusCode::CONFLICT, msg)
    }

    fn or_500(self, msg: &str) -> Result<T, Response> {
        self.or_status(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }

    fn or_503(self, msg: &str) -> Result<T, Response> {
        self.or_status(StatusCode::SERVICE_UNAVAILABLE, msg)
    }

    fn or_status(self, status: StatusCode, msg: &str) -> Result<T, Response> {
        self.map_err(|e| {
            if status.is_server_error() {
                tracing::error!(error = %e, "{msg}");
            } else {
                tracing::debug!(error = %e, "{msg}");
            }
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
