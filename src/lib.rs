pub mod auth;
pub mod cli;
pub mod handlers;
pub mod middleware;
pub mod server;
pub mod utils;
pub mod webdav;

pub use auth::{AuthConfig, build_auth_config};
pub use cli::{Cli, ShadowFileArg};
pub use server::tls::TlsConfig;
pub use server::{AppState, ServerConfig, start_server};

#[cfg(debug_assertions)]
pub const DEFAULT_LOG_LEVEL: &str = "debug";
#[cfg(not(debug_assertions))]
pub const DEFAULT_LOG_LEVEL: &str = "info";

#[cfg(test)]
pub(crate) fn make_test_state(root: &std::path::Path) -> std::sync::Arc<AppState> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::RwLock;

    let b = root.to_path_buf();
    let c = b.canonicalize().unwrap_or_else(|_| b.clone());
    Arc::new(AppState {
        root_dir: b.clone(),
        root_canonical: c,
        auth_config: Arc::new(AuthConfig::new()),
        dead_props: Arc::new(RwLock::new(HashMap::new())),
        locks: Arc::new(RwLock::new(HashMap::new())),
    })
}
