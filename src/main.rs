use std::path::PathBuf;

use clap::Parser;
use env_logger::Env;
use rshs::Cli;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let auth_config = cli.to_auth_config();
    let config = rshs::ServerConfig::new(
        cli.host,
        cli.port,
        PathBuf::from(cli.root_dir),
        auth_config,
        cli.is_dav,
    );
    rshs::start_server(config).await
}
