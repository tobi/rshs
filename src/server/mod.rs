pub mod auth_basic;
pub mod http_server;
pub mod shadow;
pub(crate) mod time_util;
pub mod tls;
pub mod webdav;

use std::path::PathBuf;

use actix_web::{App, HttpServer, web};
use actix_web_httpauth::middleware::HttpAuthentication;
use tracing_actix_web::TracingLogger;

use crate::middleware::health_check;
use auth_basic::AuthConfig;
use tls::TlsConfig;

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub root_dir: PathBuf,
    pub auth_config: AuthConfig,
    pub tls_config: Option<TlsConfig>,
}

impl ServerConfig {
    pub fn new(
        host: String,
        port: u16,
        root_dir: PathBuf,
        auth_config: AuthConfig,
        tls_config: Option<TlsConfig>,
    ) -> Self {
        Self {
            root_dir,
            host,
            port,
            auth_config,
            tls_config,
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

    let ls_config = match &config.tls_config {
        Some(tls_config) => {
            tracing::info!(
                root_dir = %root_dir.display(),
                addr = %addr,
                cert = %tls_config.cert_path,
                key = %tls_config.key_path,
                "starting server with TLS"
            );
            Some(tls_config.load()?)
        }
        None => {
            tracing::info!(
                root_dir = %root_dir.display(),
                addr = %addr,
                "starting server"
            );
            None
        }
    };

    let dav_handler = webdav::create_dav_handler(&root_dir);

    match ls_config {
        Some(ls_config) => {
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
                .bind_rustls_0_23(&addr, ls_config)?
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
                .bind_rustls_0_23(&addr, ls_config)?
                .run()
                .await
            }
        }
        None => {
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
    }
}
