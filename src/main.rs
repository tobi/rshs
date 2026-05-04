use clap::Parser;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style("RSHS_LOG_STYLE")
            .filter_or("RSHS_LOG", "info"),
    )
    .init();

    let cli = rshs::Cli::parse();
    let auth_config = cli.to_auth_config();

    rshs::start_server(rshs::ServerConfig::new(
        cli.host,
        cli.port,
        PathBuf::from(cli.root_dir),
        auth_config,
    ))
    .await
}
