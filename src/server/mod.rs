pub mod webdav;

use actix_web::{App, HttpServer, middleware::Logger, web};
use std::path::PathBuf;

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

    let dav_handler = webdav::create_dav_handler(&root_dir);

    log::info!("Serving {} on http://{}", root_dir.display(), addr);

    HttpServer::new(move || {
        let dav = dav_handler.clone();
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(dav))
            .default_service(web::to(webdav::dav_route))
    })
    .bind(&addr)?
    .run()
    .await
}
