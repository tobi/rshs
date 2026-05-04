use crate::server::auth_basic::AuthConfig;
use clap::Parser;

/// Simple HTTP/WebDAV Server
#[derive(Parser)]
#[command(name = "rshs")]
pub struct Cli {
    /// Root directory to serve
    pub root_dir: String,

    /// Host address to bind to
    #[arg(short = 'H', long, default_value = "0.0.0.0")]
    pub host: String,

    /// Port to bind to
    #[arg(short, long, default_value = "8080")]
    pub port: u16,

    /// Basic Auth credentials in format username:password (can be repeated)
    #[arg(
        short = 'U',
        long,
        value_name = "USER:PASS",
        verbatim_doc_comment,
        value_delimiter = ';',
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
}
