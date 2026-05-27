//! HTTP Basic Authentication middleware.
//! Automatically becomes a no-op when no users are configured in `AuthState`.

use std::sync::Arc;

use axum::{body::Body, http::StatusCode, middleware::Next, response::Response};
use base64::{Engine as _, engine::general_purpose};

use crate::auth::{AuthState, hash_auth_header};

/// Validates HTTP Basic Authentication credentials against the configured `AuthState`.
/// Skips authentication entirely when no users are configured (backward compatible).
///
/// For SHA-512 crypt credentials, uses an auth cache to avoid re-verifying the
/// expensive password hash on every request. See [`AuthState::validate_cached`].
///
/// Returns `401 Unauthorized` with `WWW-Authenticate: Basic realm="rshs"` on failure.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<AuthState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    if state.is_empty() {
        return Ok(next.run(req).await);
    }

    let (username, password) = match parse_basic_auth(req.headers()) {
        Some(creds) => creds,
        None => {
            return Err(unauthorized());
        }
    };

    let auth_header = req.headers().get("authorization");
    let header_hash = auth_header
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Basic "))
        .map(hash_auth_header);

    let Some(header_hash) = header_hash else {
        return Err(unauthorized());
    };

    if state
        .validate_cached(&username, &password, header_hash)
        .await
    {
        tracing::debug!(user = %username, "authentication succeeded");
        Ok(next.run(req).await)
    } else {
        tracing::warn!(user = %username, "authentication failed");
        Err(unauthorized())
    }
}

fn parse_basic_auth(headers: &axum::http::HeaderMap) -> Option<(String, String)> {
    let header = headers.get("authorization")?.to_str().ok()?;
    let stripped = header.strip_prefix("Basic ")?;
    let decoded = general_purpose::STANDARD.decode(stripped).ok()?;
    let decoded = std::str::from_utf8(&decoded).ok()?;
    let (user, pass) = decoded.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", r#"Basic realm="rshs""#)
        .body(Body::empty())
        .unwrap()
}
