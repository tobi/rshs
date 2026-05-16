pub mod tls;

use std::fs;
use std::io::{self, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::middleware as axum_mw;
use axum::response::IntoResponse;
use axum::{
    Router,
    extract::State,
    http::{Method, StatusCode},
    routing::any,
};
use tower_http::trace::TraceLayer;

use crate::auth::AuthConfig;
use crate::handlers::{http, locks, webdav as webdav_handler};
use crate::middleware;

#[derive(Clone)]
pub struct AppState {
    pub root_dir: PathBuf,
    pub root_canonical: PathBuf,
    pub auth_config: Arc<AuthConfig>,
    pub dead_props: Arc<tokio::sync::RwLock<crate::webdav::DeadPropertyStore>>,
    pub locks: Arc<tokio::sync::RwLock<crate::webdav::LockStore>>,
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
    let method = req.method();

    if method == Method::GET || method == Method::HEAD {
        return http::handle_get_head(State(state), req).await;
    }
    if method == Method::PUT {
        return http::handle_put(State(state), req).await;
    }
    if method == Method::DELETE {
        return http::handle_delete(State(state), req).await;
    }
    if method == Method::OPTIONS {
        return http::handle_options().await;
    }
    if method == *crate::webdav::M_PROPFIND {
        return webdav_handler::handle_propfind(State(state), req).await;
    }
    if method == *crate::webdav::M_MKCOL {
        return webdav_handler::handle_mkcol(State(state), req).await;
    }
    if method == *crate::webdav::M_COPY {
        return webdav_handler::handle_copy(State(state), req).await;
    }
    if method == *crate::webdav::M_MOVE {
        return webdav_handler::handle_move(State(state), req).await;
    }
    if method == *crate::webdav::M_PROPPATCH {
        return webdav_handler::handle_proppatch(State(state), req).await;
    }
    if method == *crate::webdav::M_LOCK {
        return locks::handle_lock(State(state), req).await;
    }
    if method == *crate::webdav::M_UNLOCK {
        return locks::handle_unlock(State(state), req).await;
    }

    StatusCode::NOT_IMPLEMENTED.into_response()
}

pub fn app(config: &ServerConfig) -> Router {
    let state = Arc::new(AppState {
        root_dir: config.root_dir.clone(),
        root_canonical: fs::canonicalize(&config.root_dir)
            .unwrap_or_else(|_| config.root_dir.clone()),
        auth_config: Arc::new(config.auth_config.clone()),
        dead_props: Arc::new(tokio::sync::RwLock::new(
            crate::webdav::DeadPropertyStore::new(),
        )),
        locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
    });

    Router::new()
        .fallback(any(dispatch))
        .layer(TraceLayer::new_for_http())
        .layer(axum_mw::from_fn_with_state(
            state.auth_config.clone(),
            middleware::auth::auth_middleware,
        ))
        .layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::lock::lock_enforce,
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
