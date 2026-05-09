use crate::server::auth_basic::AuthConfig;
use crate::server::tls::TlsConfig;
use clap::Parser;

use crate::DEFAULT_LOG_LEVEL;

/// Arguments for shadow file access mode
#[derive(Debug, Clone)]
pub struct ShadowFileArg {
    /// Path to the shadow file
    pub path: String,
    /// Whether the file is writable (:rw suffix)
    pub writable: bool,
}

/// Simple HTTP/WebDAV Server
#[derive(Parser)]
#[command(
    name = "rshs",
    version = env!("CARGO_PKG_VERSION"),
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
}

impl Cli {
    pub fn effective_port(&self) -> u16 {
        self.port
            .unwrap_or(if self.tls_cert.is_some() { 8443 } else { 8080 })
    }

    pub fn to_tls_config(&self) -> Option<TlsConfig> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Some(TlsConfig::new(cert.clone(), key.clone())),
            _ => None,
        }
    }

    pub fn to_shadow_file_arg(&self) -> Option<ShadowFileArg> {
        self.shadow_file.as_ref().map(|s| {
            if let Some(path) = s.strip_suffix(":rw") {
                ShadowFileArg {
                    path: path.to_string(),
                    writable: true,
                }
            } else if let Some(path) = s.strip_suffix(":ro") {
                ShadowFileArg {
                    path: path.to_string(),
                    writable: false,
                }
            } else {
                ShadowFileArg {
                    path: s.clone(),
                    writable: true,
                }
            }
        })
    }

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

    pub fn log_level(&self) -> &str {
        if self.quiet {
            "off"
        } else {
            match self.verbose {
                0 => DEFAULT_LOG_LEVEL,
                1 => "debug",
                _ => "trace",
            }
        }
    }
}
