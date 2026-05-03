use std::path::PathBuf;

use clap::Parser;
use rshs::Cli;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let config = rshs::ServerConfig::new(cli.host, cli.port, PathBuf::from(cli.root_dir));
    rshs::start_server(config).await
}
