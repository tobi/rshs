use std::sync::Arc;

use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::webdav;

const WRITE_METHODS: &[&str] = &["PUT", "DELETE", "MKCOL", "PROPPATCH"];

pub async fn lock_enforce(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    let method = req.method().as_str();

    if !WRITE_METHODS.contains(&method) {
        return Ok(next.run(req).await);
    }

    let request_path = req.uri().path().to_owned();
    let tokens = webdav::parse_if_header(req.headers());
    let locks = state.locks.read().await;

    // Build the canonical path to match lock store keys
    let canonical = match state
        .root_canonical
        .join(request_path.trim_start_matches('/'))
    {
        p if p.starts_with(&state.root_canonical) => p,
        _ => return Ok(next.run(req).await),
    };

    // Canonicalize: check if this path or any parent is locked
    if let Some(lock_infos) = locks.get(&canonical) {
        if lock_infos.iter().any(|l| !tokens.contains(&l.token)) {
            tracing::debug!(path = %canonical.display(), "resource locked, rejecting write");
            return Err(StatusCode::LOCKED.into_response());
        }
    }

    drop(locks);
    Ok(next.run(req).await)
}
