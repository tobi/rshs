use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use rshs::DEFAULT_LOG_LEVEL;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = rshs::Cli::parse();

    let filter = if cli.quiet {
        EnvFilter::new("off")
    } else if cli.verbose >= 2 {
        EnvFilter::new("trace")
    } else if cli.verbose >= 1 {
        EnvFilter::new("debug")
    } else if let Ok(f) = std::env::var("RSHS_LOG") {
        EnvFilter::new(f)
    } else {
        EnvFilter::new(DEFAULT_LOG_LEVEL)
    };

    let ansi = match std::env::var("RSHS_LOG_STYLE").as_deref() {
        Ok("always") => true,
        Ok("never") => false,
        _ => io::stderr().is_terminal(),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .with_writer(io::stderr)
        .init();

    let _ = tracing_log::LogTracer::init();

    let auth_config = rshs::build_auth_config(&cli);

    let port = cli.effective_port();
    let tls_config = cli.to_tls_config();
    let host = cli.host;
    let root_dir = PathBuf::from(cli.root_dir);

    rshs::start_server(rshs::ServerConfig::new(
        root_dir,
        host,
        port,
        tls_config,
        auth_config,
    ))
    .await
}
