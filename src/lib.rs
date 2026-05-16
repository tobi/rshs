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
