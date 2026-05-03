use std::path::PathBuf;

use actix_web::{App, HttpServer, web};
use dav_server::{
    DavHandler, actix::DavRequest, actix::DavResponse, fakels::FakeLs, localfs::LocalFs,
};

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub root_dir: PathBuf,
}

impl ServerConfig {
    pub fn new(host: String, port: u16, root_dir: PathBuf) -> Self {
        Self {
            host,
            port,
            root_dir,
        }
    }
}

pub async fn start_server(config: ServerConfig) -> std::io::Result<()> {
    let root_dir = config.root_dir.clone();
    let addr = format!("{}:{}", config.host, config.port);

    let dav_handler = DavHandler::builder()
        .filesystem(LocalFs::new(&root_dir, false, false, false))
        .locksystem(FakeLs::new())
        .build_handler();

    println!("Serving {} on http://{}", root_dir.display(), addr);

    HttpServer::new(move || {
        let dav = dav_handler.clone();
        App::new()
            .app_data(web::Data::new(dav))
            .default_service(web::to(dav_route))
    })
    .bind(&addr)?
    .run()
    .await
}

async fn dav_route(req: DavRequest, dav: web::Data<DavHandler>) -> DavResponse {
    dav.handle(req.request).await.into()
}
