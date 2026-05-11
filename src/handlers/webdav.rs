use std::sync::Arc;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use dav_server::{DavHandler, fakels::FakeLs, localfs::LocalFs};

use crate::server::AppState;

pub fn create_dav_handler(root_dir: &std::path::Path) -> DavHandler {
    DavHandler::builder()
        .filesystem(LocalFs::new(root_dir, true, false, false))
        .locksystem(FakeLs::new())
        .build_handler()
}

pub async fn dav_route(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    tracing::debug!(method = %method, path = %path, "WebDAV request");
    state.dav_handler.handle(req).await.into_response()
}
