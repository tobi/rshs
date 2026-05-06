use actix_web::web;
use dav_server::{
    DavHandler, actix::DavRequest, actix::DavResponse, fakels::FakeLs, localfs::LocalFs,
};
use std::path::Path;

pub fn create_dav_handler(root_dir: &Path) -> DavHandler {
    DavHandler::builder()
        .filesystem(LocalFs::new(root_dir, false, false, false))
        .locksystem(FakeLs::new())
        .build_handler()
}

pub async fn dav_route(req: DavRequest, dav: web::Data<DavHandler>) -> DavResponse {
    tracing::debug!(
        method = %req.request.method().as_str(),
        path = %req.request.uri().path(),
        "WebDAV request"
    );
    dav.handle(req.request).await.into()
}
