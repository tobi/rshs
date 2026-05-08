pub mod auth_basic;
pub mod http_server;
pub mod shadow;
pub(crate) mod time_util;
pub mod webdav;

use actix_web::{App, HttpServer, web};
use actix_web_httpauth::middleware::HttpAuthentication;
use std::path::PathBuf;
use tracing_actix_web::TracingLogger;

use crate::middleware::health_check;
use auth_basic::AuthConfig;

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub root_dir: PathBuf,
    pub auth_config: AuthConfig,
}

impl ServerConfig {
    pub fn new(host: String, port: u16, root_dir: PathBuf, auth_config: AuthConfig) -> Self {
        Self {
            root_dir,
            host,
            port,
            auth_config,
        }
    }
}

fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/{path:.*}", web::get().to(http_server::handle))
        .route("/{path:.*}", web::head().to(http_server::handle))
        .default_service(web::to(webdav::dav_route));
}

pub async fn start_server(config: ServerConfig) -> std::io::Result<()> {
    let root_dir = config.root_dir;
    let addr = format!("{}:{}", config.host, config.port);
    let auth_config = config.auth_config;

    tracing::info!(root_dir = %root_dir.display(), addr = %addr, "starting server");

    let dav_handler = webdav::create_dav_handler(&root_dir);

    if !auth_config.is_empty() {
        HttpServer::new(move || {
            let auth_config = auth_config.clone();
            let dav = dav_handler.clone();
            let root_dir = root_dir.clone();
            App::new()
                .wrap(TracingLogger::default())
                .wrap(HttpAuthentication::basic(auth_basic::auth_validator))
                .wrap(health_check::HealthCheck)
                .app_data(web::Data::new(auth_config))
                .app_data(web::Data::new(dav))
                .app_data(web::Data::new(root_dir))
                .configure(configure_routes)
        })
        .bind(&addr)?
        .run()
        .await
    } else {
        HttpServer::new(move || {
            let dav = dav_handler.clone();
            let root_dir = root_dir.clone();
            App::new()
                .wrap(TracingLogger::default())
                .wrap(health_check::HealthCheck)
                .app_data(web::Data::new(dav))
                .app_data(web::Data::new(root_dir))
                .configure(configure_routes)
        })
        .bind(&addr)?
        .run()
        .await
    }
}
