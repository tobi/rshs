pub mod auth_basic;
pub mod http_server;
pub mod webdav;

use actix_web::{App, HttpServer, middleware::Logger, web};
use actix_web_httpauth::middleware::HttpAuthentication;
use std::path::PathBuf;

use auth_basic::AuthConfig;

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub root_dir: PathBuf,
    pub auth_config: AuthConfig,
    pub is_dav: bool,
}

impl ServerConfig {
    pub fn new(
        host: String,
        port: u16,
        root_dir: PathBuf,
        auth_config: AuthConfig,
        is_dav: bool,
    ) -> Self {
        Self {
            host,
            port,
            root_dir,
            auth_config,
            is_dav,
        }
    }
}

pub async fn start_server(config: ServerConfig) -> std::io::Result<()> {
    let root_dir = config.root_dir;
    let addr = format!("{}:{}", config.host, config.port);
    let auth_config = config.auth_config;
    let is_dav = config.is_dav;

    let mode = if is_dav { "WebDAV" } else { "HTTP" };
    log::info!(
        "Serving {} on http://{} ({mode} mode)",
        root_dir.display(),
        addr
    );

    if is_dav {
        let dav_handler = webdav::create_dav_handler(&root_dir);
        if !auth_config.is_empty() {
            HttpServer::new(move || {
                let auth_config = auth_config.clone();
                let dav = dav_handler.clone();
                App::new()
                    .wrap(Logger::default())
                    .wrap(HttpAuthentication::basic(auth_basic::auth_validator))
                    .app_data(web::Data::new(auth_config))
                    .app_data(web::Data::new(dav))
                    .default_service(web::to(webdav::dav_route))
            })
            .bind(&addr)?
            .run()
            .await
        } else {
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
    } else {
        let root_dir = PathBuf::from(&root_dir);
        if !auth_config.is_empty() {
            HttpServer::new(move || {
                let auth_config = auth_config.clone();
                let root_dir = root_dir.clone();
                App::new()
                    .wrap(Logger::default())
                    .wrap(HttpAuthentication::basic(auth_basic::auth_validator))
                    .app_data(web::Data::new(auth_config))
                    .app_data(web::Data::new(root_dir))
                    .default_service(web::to(http_server::handle))
            })
            .bind(&addr)?
            .run()
            .await
        } else {
            HttpServer::new(move || {
                let root_dir = root_dir.clone();
                App::new()
                    .wrap(Logger::default())
                    .app_data(web::Data::new(root_dir))
                    .default_service(web::to(http_server::handle))
            })
            .bind(&addr)?
            .run()
            .await
        }
    }
}
