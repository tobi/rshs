use clap::Parser;
use env_logger::{Builder, Env};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = rshs::Cli::parse();

    let mut builder = Builder::from_env(Env::new().write_style("RSHS_LOG_STYLE"));

    if cli.quiet {
        builder.filter_level(log::LevelFilter::Off);
    } else if cli.verbose >= 2 {
        builder.filter_level(log::LevelFilter::Trace);
    } else if cli.verbose >= 1 {
        builder.filter_level(log::LevelFilter::Debug);
    } else if let Ok(filter_str) = std::env::var("RSHS_LOG") {
        builder.parse_filters(&filter_str);
    } else {
        builder.filter_level(log::LevelFilter::Info);
    }

    builder.init();

    let auth_config = cli.to_auth_config();

    rshs::start_server(rshs::ServerConfig::new(
        cli.host,
        cli.port,
        PathBuf::from(cli.root_dir),
        auth_config,
    ))
    .await
}
