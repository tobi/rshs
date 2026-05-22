//! CLI argument parsing via `clap` derive, plus configuration builders that convert
//! CLI values into the server's typed config structs.

use clap::Parser;

use crate::DEFAULT_LOG_LEVEL;
use crate::auth::{AuthConfig, ShadowFileArg};
use crate::server::tls::TlsConfig;

/// Simple HTTP/WebDAV Server
#[derive(Parser)]
#[command(
    name = "rshs", version = env!("CARGO_PKG_VERSION"),
    after_help = concat!(
        "Logging environment variables:\n",
        "  RSHS_LOG          Tracing filter (e.g. info, rshs=debug, rshs[status=500]=trace)\n",
        "                    Only used when no -v/-q flags are given\n",
        "                    Supports per-target and per-field filtering\n",
        "  RSHS_LOG_STYLE    Log style (always, never, auto), controls ANSI color output\n",
        "                    Defaults to auto (enabled when output is a terminal)",
    ),
)]
pub struct Cli {
    /// Root directory to serve
    #[arg(default_value = ".", env = "RSHS_ROOT_DIR")]
    pub root_dir: String,

    /// Host address to bind to
    #[arg(short = 'H', long, default_value = "0.0.0.0", env = "RSHS_HOST")]
    pub host: String,

    /// Port to bind to (default: 8080, or 8443 with TLS)
    #[arg(short, long, env = "RSHS_PORT")]
    pub port: Option<u16>,

    /// TLS certificate file path (PEM format)
    #[arg(long = "tls-cert", env = "RSHS_TLS_CERT", requires = "tls_key")]
    pub tls_cert: Option<String>,

    /// TLS private key file path (PEM format)
    #[arg(long = "tls-key", env = "RSHS_TLS_KEY", requires = "tls_cert")]
    pub tls_key: Option<String>,

    /// Increase log verbosity (-v = debug, -vv = trace)
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, conflicts_with = "quiet")]
    pub verbose: u8,

    /// Suppress all log output
    #[arg(short = 'q', long = "quiet", conflicts_with = "verbose")]
    pub quiet: bool,

    /// Basic Auth credentials in format username:password (can be repeated)
    #[arg(
        short = 'u',
        long = "user",
        value_name = "USER:PASS",
        verbatim_doc_comment,
        value_delimiter = ';',
        hide_env_values = true,
        env = "RSHS_USERS"
    )]
    pub users: Vec<String>,

    /// Path to shadow file for persistent auth (PATH[:rw|:ro], default :rw)
    #[arg(
        short = 'S',
        long = "shadow-file",
        value_name = "PATH[:rw|:ro]",
        env = "RSHS_SHADOW_FILE"
    )]
    pub shadow_file: Option<String>,

    /// Write CLI credentials into the shadow file (requires --shadow-file :rw)
    #[arg(short = 'W', long = "shadow-write", requires = "shadow_file")]
    pub shadow_write: bool,

    /// Default WebDAV lock timeout in seconds, 0 for unlimited
    #[arg(
        long = "lock-timeout",
        default_value = "300",
        env = "RSHS_LOCK_TIMEOUT",
        value_parser = clap::value_parser!(u64)
    )]
    pub lock_timeout: u64,
}

impl Cli {
    /// Returns the effective port: the explicit `--port` value, or the default
    /// (`8080` for HTTP, `8443` for HTTPS when TLS is configured).
    pub fn effective_port(&self) -> u16 {
        self.port
            .unwrap_or(if self.tls_cert.is_some() { 8443 } else { 8080 })
    }

    /// Builds a `TlsConfig` if both `--tls-cert` and `--tls-key` were provided.
    pub fn to_tls_config(&self) -> Option<TlsConfig> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Some(TlsConfig::new(cert.clone(), key.clone())),
            _ => None,
        }
    }

    /// Parses the `--shadow-file` value into a `ShadowFileArg` (path + read/write mode).
    pub fn to_shadow_file_arg(&self) -> Option<ShadowFileArg> {
        self.shadow_file
            .as_ref()
            .map(|s| ShadowFileArg::from_arg(s))
    }

    /// Builds an `AuthConfig` from `--user` (`username:password`) entries.
    pub fn to_auth_config(&self) -> AuthConfig {
        let mut config = AuthConfig::new();

        for entry in &self.users {
            if let Some((username, password)) = entry.split_once(':')
                && !username.is_empty()
            {
                config.add_user(username, password);
            }
        }

        config
    }

    /// Resolves the log level: `-q` → `"off"`, `-v` → `"debug"`, `-vv` → `"trace"`,
    /// otherwise the `RSHS_LOG` env var or `DEFAULT_LOG_LEVEL`.
    pub fn log_level(&self) -> String {
        if self.quiet {
            "off".into()
        } else {
            match self.verbose {
                0 => std::env::var("RSHS_LOG").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.into()),
                1 => "debug".into(),
                _ => "trace".into(),
            }
        }
    }
}
