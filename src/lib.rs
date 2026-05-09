pub mod cli;
pub mod middleware;
pub mod server;

pub use cli::{Cli, ShadowFileArg};
pub use server::auth_basic::AuthConfig;
pub use server::http_server;
pub use server::shadow;
pub use server::tls::TlsConfig;
pub use server::webdav as dav;
pub use server::{ServerConfig, start_server};

#[cfg(debug_assertions)]
pub const DEFAULT_LOG_LEVEL: &str = "debug";
#[cfg(not(debug_assertions))]
pub const DEFAULT_LOG_LEVEL: &str = "info";
