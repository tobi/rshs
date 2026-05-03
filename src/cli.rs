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
}
