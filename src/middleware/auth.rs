use std::sync::Arc;

use axum::{body::Body, http::StatusCode, middleware::Next, response::Response};
use base64::{Engine as _, engine::general_purpose};

use crate::auth::AuthConfig;

pub async fn auth_middleware(
    axum::extract::State(auth_config): axum::extract::State<Arc<AuthConfig>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    if auth_config.is_empty() {
        return Ok(next.run(req).await);
    }

    let (username, password) = match parse_basic_auth(req.headers()) {
        Some(creds) => creds,
        None => {
            return Err(unauthorized());
        }
    };

    if auth_config.validate(&username, &password) {
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
