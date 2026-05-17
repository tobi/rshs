use std::sync::Arc;

use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::webdav;

pub async fn lock_enforce(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    let method = req.method().as_str();

    if !matches!(
        method,
        "PUT" | "DELETE" | "MKCOL" | "PROPPATCH" | "MOVE" | "COPY"
    ) {
        return Ok(next.run(req).await);
    }

    let request_path = req.uri().path().trim_end_matches('/').to_owned();
    let tokens = webdav::parse_if_header(req.headers());

    // Check source path against lock store
    if let Ok(src) = state.resolve_and_guard(&request_path).await {
        let locks = state.locks.read().await;
        if let Some(infos) = locks.get(&src) {
            if infos.iter().any(|l| !tokens.contains(&l.token)) {
                tracing::debug!(path = %src.display(), "resource locked, rejecting write");
                return Err(StatusCode::LOCKED.into_response());
            }
        }
        drop(locks);
    }

    // For COPY/MOVE, additionally check destination path
    if method == "COPY" || method == "MOVE" {
        if let Some(dest) = webdav::parse_destination(req.headers()) {
            if let Ok(dest_path) = state.resolve_and_guard(dest.trim_end_matches('/')).await {
                let locks = state.locks.read().await;
                if let Some(infos) = locks.get(&dest_path) {
                    if infos.iter().any(|l| !tokens.contains(&l.token)) {
                        tracing::debug!(path = %dest_path.display(), "destination locked, rejecting COPY/MOVE");
                        return Err(StatusCode::LOCKED.into_response());
                    }
                }
                drop(locks);
            }
        }
    }

    Ok(next.run(req).await)
}
