pub mod tls;

use std::fs;
use std::io::{self, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::middleware as axum_mw;
use axum::{Router, extract::State, http::Method, routing::any};
use dav_server::DavHandler;
use tower_http::trace::TraceLayer;

use crate::auth::AuthConfig;
use crate::handlers::{file, webdav};
use crate::middleware;

#[derive(Clone)]
pub struct AppState {
    pub root_dir: PathBuf,
    pub root_canonical: PathBuf,
    pub dav_handler: DavHandler,
    pub auth_config: Arc<AuthConfig>,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub root_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub tls_config: Option<tls::TlsConfig>,
    pub auth_config: AuthConfig,
}

impl ServerConfig {
    pub fn new(
        root_dir: PathBuf,
        host: String,
        port: u16,
        tls_config: Option<tls::TlsConfig>,
        auth_config: AuthConfig,
    ) -> Self {
        Self {
            root_dir,
            host,
            port,
            tls_config,
            auth_config,
        }
    }
}

async fn dispatch(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    match *req.method() {
        Method::GET | Method::HEAD => file::handle(State(state), req).await,
        _ => webdav::dav_route(State(state), req).await,
    }
}

pub fn app(config: &ServerConfig) -> Router {
    let state = Arc::new(AppState {
        root_dir: config.root_dir.clone(),
        root_canonical: fs::canonicalize(&config.root_dir)
            .unwrap_or_else(|_| config.root_dir.clone()),
        dav_handler: webdav::create_dav_handler(&config.root_dir),
        auth_config: Arc::new(config.auth_config.clone()),
    });

    Router::new()
        .fallback(any(dispatch))
        .layer(TraceLayer::new_for_http())
        .layer(axum_mw::from_fn_with_state(
            state.auth_config.clone(),
            middleware::auth::auth_middleware,
        ))
        .layer(middleware::health::HealthCheck)
        .with_state(state)
}

pub async fn start_server(config: ServerConfig) -> io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    let router = app(&config);

    match &config.tls_config {
        Some(tls_config) => {
            let listener = tls::TlsListener::bind(addr, tls_config.load()?).await?;
            tracing::info!(
                addr = %addr, cert = %tls_config.cert_path, key = %tls_config.key_path,
                "starting HTTPS server"
            );
            axum::serve(listener, router).await.map_err(Error::other)?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(addr = %addr, "starting HTTP server");
            axum::serve(listener, router).await.map_err(Error::other)?;
        }
    }

    Ok(())
}
