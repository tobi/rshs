use crate::server::auth_basic::AuthConfig;
use clap::Parser;

/// Simple HTTP/WebDAV Server
#[derive(Parser)]
#[command(name = "rshs", version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Root directory to serve
    #[arg(default_value = ".", env = "RSHS_ROOT_DIR")]
    pub root_dir: String,

    /// Host address to bind to
    #[arg(short = 'H', long, default_value = "0.0.0.0", env = "RSHS_HOST")]
    pub host: String,

    /// Port to bind to
    #[arg(short, long, default_value = "8080", env = "RSHS_PORT")]
    pub port: u16,

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
}

impl Cli {
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
                0 => "info",
                1 => "debug",
                _ => "trace",
            }
        }
    }
}
