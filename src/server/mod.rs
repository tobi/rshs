//! Router construction, request dispatch, and server startup for both HTTP and HTTPS.
//! Also provides the lock-cleanup background task.

pub(crate) mod tls;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use derive_new::new;
use tokio::signal;
use tokio::sync::{Notify, RwLock};

use crate::auth::AuthConfig;
use crate::handlers::{http, locks, webdav as webdav_handler};
use crate::utils::path::{self, ResolveTargetError};
use crate::webdav::{DeadPropertyStore, LockStore, Method};

/// Shared application state passed to every handler and middleware.
///
/// Holds the root directory, auth config, WebDAV dead property store, and lock store.
/// All fields are behind `Arc` for cheap cloning.
#[derive(Clone)]
pub struct AppState {
    pub auth_config: Arc<AuthConfig>,
    pub root_dir: PathBuf,
    pub root_canonical: PathBuf,
    pub dead_props: Arc<RwLock<DeadPropertyStore>>,
    pub locks: Arc<RwLock<LockStore>>,
    pub canonical_cache: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
    pub lock_timeout: Duration,
}

impl AppState {
    pub fn new(root: PathBuf, auth_config: AuthConfig, lock_timeout: Duration) -> Self {
        let root_canonical = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        Self {
            auth_config: Arc::new(auth_config),
            root_dir: root,
            root_canonical,
            dead_props: Arc::new(RwLock::new(DeadPropertyStore::new())),
            locks: Arc::new(RwLock::new(LockStore::new())),
            canonical_cache: Arc::new(Mutex::new(HashMap::new())),
            lock_timeout,
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
        path::resolve_and_guard(
            &self.root_dir,
            &self.root_canonical,
            request_path,
            &self.canonical_cache,
        )
        .await
    }
}

/// Configuration for starting the server — root directory, bind address,
/// optional TLS, authentication, and default lock timeout.
#[derive(Clone, new)]
pub struct ServerConfig {
    pub root_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub tls_config: Option<tls::TlsConfig>,
    pub auth_config: AuthConfig,
    pub lock_timeout: u64,
}

/// Builds the axum router with all middleware layers, then starts the HTTP or HTTPS
/// server. Also spawns a background task to prune expired WebDAV locks every 30 seconds.
pub async fn start_server(config: ServerConfig) -> io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;

    let auth_config = config.auth_config;
    let root = config.root_dir;
    let lock_timeout = if config.lock_timeout == 0 {
        Duration::ZERO // A zero lock timeout means locks never expire
    } else {
        Duration::from_secs(config.lock_timeout)
    };

    let state = Arc::new(AppState::new(root, auth_config, lock_timeout));

    let router = make_router(state.clone());

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
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(Error::other)?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(addr = %addr, "starting HTTP server");
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .map_err(Error::other)?;
        }
    }

    cleanup_notify.notify_one();

    let _ = cleanup_handle.await;

    Ok(())
}

/// Build the full middleware stack and request dispatch router from shared state.
///
/// This produces the same Router used by [`start_server`], enabling integration
/// testing without binding a TCP port. Layers are applied from inside out:
/// `HealthCheck` (outermost) → `LockEnforce` → `Auth` → `TraceLayer` → dispatch.
pub fn make_router(state: Arc<AppState>) -> Router {
    use crate::middleware::{auth, health, lock};
    use axum::middleware::from_fn_with_state;
    use tower_http::trace::TraceLayer;

    let auth_config = state.auth_config.clone();
    let auth_mw = from_fn_with_state(auth_config, auth::auth_middleware);
    let lock_mw = from_fn_with_state(state.clone(), lock::lock_enforce);
    let health_check_mw = health::HealthCheck;

    Router::new()
        .fallback(any(dispatch))
        .layer(TraceLayer::new_for_http())
        .layer(auth_mw)
        .layer(lock_mw)
        .layer(health_check_mw)
        .with_state(state)
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
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
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

#[cfg(unix)]
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to listen for Ctrl+C");
        tracing::info!("received Ctrl+C, shutting down gracefully...");
    };

    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
        tracing::info!("received SIGTERM, shutting down gracefully...");
    };

    tokio::select! {
        () = sigterm => {},
        () = ctrl_c => {},
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    signal::ctrl_c().await.expect("failed to listen for Ctrl+C");
    tracing::info!("received shutdown signal, shutting down gracefully...");
}
