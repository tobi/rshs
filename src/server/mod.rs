pub(crate) mod tls;

use std::fs;
use std::io::{self, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware as axum_mw;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use derive_new::new;
use tokio::sync::{Notify, RwLock};
use tower_http::trace::TraceLayer;

use crate::auth::AuthConfig;
use crate::handlers::{http, locks, webdav as webdav_handler};
use crate::middleware;
use crate::utils::path::{self, ResolveTargetError};
use crate::webdav::{DeadPropertyStore, LockStore, Method};

#[derive(Clone)]
pub struct AppState {
    pub auth_config: Arc<AuthConfig>,
    pub root_dir: PathBuf,
    pub root_canonical: PathBuf,
    pub dead_props: Arc<RwLock<DeadPropertyStore>>,
    pub locks: Arc<RwLock<LockStore>>,
}

impl AppState {
    pub fn new(root_dir: PathBuf, auth_config: AuthConfig) -> Self {
        let root_canonical = fs::canonicalize(&root_dir).unwrap_or_else(|_| root_dir.clone());
        Self {
            auth_config: Arc::new(auth_config),
            root_dir,
            root_canonical,
            dead_props: Arc::new(RwLock::new(DeadPropertyStore::new())),
            locks: Arc::new(RwLock::new(LockStore::new())),
        }
    }

    pub(crate) async fn resolve_existing(&self, request_path: &str) -> Option<PathBuf> {
        path::resolve_existing(&self.root_dir, &self.root_canonical, request_path).await
    }

    pub(crate) fn resolve_write_target(&self, request_path: &str) -> Option<PathBuf> {
        path::resolve_write_target(&self.root_dir, request_path)
    }

    pub(crate) async fn resolve_and_guard(
        &self,
        request_path: &str,
    ) -> Result<PathBuf, ResolveTargetError> {
        path::resolve_and_guard(&self.root_dir, &self.root_canonical, request_path).await
    }
}

#[derive(Clone, new)]
pub struct ServerConfig {
    pub root_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub tls_config: Option<tls::TlsConfig>,
    pub auth_config: AuthConfig,
}

pub async fn start_server(config: ServerConfig) -> io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;

    let state = Arc::new(AppState::new(
        config.root_dir.clone(),
        config.auth_config.clone(),
    ));

    let router = Router::new()
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
        .with_state(state.clone());

    let cleanup_notify = Arc::new(Notify::new());
    let task = lock_cleanup_task(state.locks.clone(), cleanup_notify.clone());
    let cleanup_handle = tokio::spawn(task);

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

    cleanup_notify.notify_one();

    let _ = cleanup_handle.await;

    Ok(())
}

async fn dispatch(State(state): State<Arc<AppState>>, req: Request) -> Response {
    match Method::try_from(req.method()) {
        Ok(Method::GET) | Ok(Method::HEAD) => http::handle_get_head(State(state), req).await,
        Ok(Method::PUT) => http::handle_put(State(state), req).await,
        Ok(Method::DELETE) => http::handle_delete(State(state), req).await,
        Ok(Method::OPTIONS) => http::handle_options().await,
        Ok(Method::PROPFIND) => webdav_handler::handle_propfind(State(state), req).await,
        Ok(Method::MKCOL) => webdav_handler::handle_mkcol(State(state), req).await,
        Ok(Method::COPY) => webdav_handler::handle_copy(State(state), req).await,
        Ok(Method::MOVE) => webdav_handler::handle_move(State(state), req).await,
        Ok(Method::PROPPATCH) => webdav_handler::handle_proppatch(State(state), req).await,
        Ok(Method::LOCK) => locks::handle_lock(State(state), req).await,
        Ok(Method::UNLOCK) => locks::handle_unlock(State(state), req).await,
        _ => StatusCode::NOT_IMPLEMENTED.into_response(),
    }
}

async fn lock_cleanup_task(locks: Arc<RwLock<LockStore>>, shutdown: Arc<Notify>) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                let mut store = locks.write().await;
                let before = store.values().map(|v| v.len()).sum::<usize>();
                store.retain(|_path, infos| {
                    infos.retain(|l| !l.is_expired());
                    !infos.is_empty()
                });
                let after = store.values().map(|v| v.len()).sum::<usize>();
                if before > after {
                    tracing::debug!(
                        removed = before - after, remaining = after, "cleanup expired locks"
                    );
                }
                drop(store);
            }
            _ = shutdown.notified() => {
                tracing::debug!("lock cleanup task shutting down");
                break;
            }
        }
    }
}
