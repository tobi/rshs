use std::path::PathBuf;

use clap::Parser;
use env_logger::Env;
use rshs::Cli;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let config = rshs::ServerConfig::new(cli.host, cli.port, PathBuf::from(cli.root_dir));
    rshs::start_server(config).await
}
