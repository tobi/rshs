use std::path::PathBuf;

use clap::Parser;

/// Simple HTTP/WebDAV Server
#[derive(Parser)]
#[command(name = "rshs")]
struct Cli {
    /// Root directory to serve
    root_dir: String,

    /// Host address to bind to
    #[arg(short = 'H', long, default_value = "0.0.0.0")]
    host: String,

    /// Port to bind to
    #[arg(short, long, default_value = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let config = rshs::ServerConfig::new(cli.host, cli.port, PathBuf::from(cli.root_dir));
    rshs::start_server(config).await
}
