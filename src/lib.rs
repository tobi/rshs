//! A simple HTTP/WebDAV server — file serving, directory listing, and full WebDAV
//! protocol support with optional TLS and Basic Auth.

/// Authentication types and shadow file management.
pub mod auth;
/// CLI argument parsing and configuration builders.
pub(crate) mod cli;
/// Request handlers for HTTP and WebDAV methods.
pub mod handlers;
/// HTML directory listing generation.
pub(crate) mod html;
/// Tower middleware layers.
pub mod middleware;
/// Router construction and server startup.
pub(crate) mod server;
/// Internal utilities (errors, path resolution, time formatting).
pub(crate) mod utils;
/// WebDAV protocol types and helpers.
pub mod webdav;

/// Authentication configuration and builder.
pub use auth::{AuthState, build_auth_state};
/// Command-line interface.
pub use cli::Cli;
/// TLS certificate/key configuration.
pub use server::tls::TlsConfig;
/// Server state, configuration, startup, and router construction.
pub use server::{AppState, ServerConfig, make_router, start_server};

#[cfg(debug_assertions)]
pub(crate) const DEFAULT_LOG_LEVEL: &str = "debug";
#[cfg(not(debug_assertions))]
pub(crate) const DEFAULT_LOG_LEVEL: &str = "info";
