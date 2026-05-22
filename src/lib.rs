pub mod auth;
pub(crate) mod cli;
pub mod handlers;
pub mod middleware;
pub(crate) mod server;
pub(crate) mod utils;
pub mod webdav;

pub use auth::{AuthConfig, build_auth_config};
pub use cli::Cli;
pub use server::tls::TlsConfig;
pub use server::{AppState, ServerConfig, start_server};

#[cfg(debug_assertions)]
pub(crate) const DEFAULT_LOG_LEVEL: &str = "debug";
#[cfg(not(debug_assertions))]
pub(crate) const DEFAULT_LOG_LEVEL: &str = "info";
