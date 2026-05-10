pub mod tls;

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::middleware as axum_mw;
use axum::{Router, extract::State, http::Method, routing::any};
use tower_http::trace::TraceLayer;

use crate::auth::AuthConfig;
use crate::handlers::{file, webdav};
use crate::middleware;

#[derive(Clone)]
pub struct AppState {
    pub root_dir: Arc<PathBuf>,
    pub root_canonical: Arc<PathBuf>,
    pub dav_handler: Arc<dav_server::DavHandler>,
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
    let auth_config = Arc::new(config.auth_config.clone());
    let root_dir = Arc::new(config.root_dir.clone());
    let root_canonical = Arc::new(
        std::fs::canonicalize(&config.root_dir).unwrap_or_else(|_| config.root_dir.clone()),
    );
    let dav_handler = Arc::new(webdav::create_dav_handler(&config.root_dir));

    let state = Arc::new(AppState {
        root_dir,
        root_canonical,
        dav_handler,
        auth_config: auth_config.clone(),
    });

    Router::new()
        .fallback(any(dispatch))
        .layer(TraceLayer::new_for_http())
        .layer(axum_mw::from_fn_with_state(
            auth_config,
            middleware::auth::auth_middleware,
        ))
        .layer(middleware::health::HealthCheck)
        .with_state(state)
}

pub async fn start_server(config: ServerConfig) -> io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let router = app(&config);

    match &config.tls_config {
        Some(tls_config) => {
            let listener = tls::TlsListener::bind(addr, tls_config.load()?).await?;
            tracing::info!(
                addr = %addr,
                cert = %tls_config.cert_path,
                key = %tls_config.key_path,
                "starting HTTPS server"
            );
            axum::serve(listener, router)
                .await
                .map_err(io::Error::other)?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(addr = %addr, "starting HTTP server");
            axum::serve(listener, router)
                .await
                .map_err(io::Error::other)?;
        }
    }

    Ok(())
}
