use clap::Parser;
use env_logger::{Builder, Env};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

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

    let auth_config = build_auth_config(&cli);

    rshs::start_server(rshs::ServerConfig::new(
        cli.host,
        cli.port,
        PathBuf::from(cli.root_dir),
        auth_config,
    ))
    .await
}

fn build_auth_config(cli: &rshs::Cli) -> rshs::AuthConfig {
    let cli_auth = cli.to_auth_config();

    let Some(shadow) = cli.to_shadow_file_arg() else {
        return cli_auth;
    };

    if cli.shadow_write && !shadow.writable {
        log::warn!(
            "shadow file {} is read-only (ro:), ignoring --shadow-write",
            shadow.path
        );
    }

    let shadow_path = Path::new(&shadow.path);

    if let Some(parent) = shadow_path.parent() {
        let parent_str = parent.as_os_str();
        if !parent_str.is_empty() && !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent) {
                log::error!("Failed to create directory {}: {e}", parent.display());
            } else {
                log::info!("Created directory {}", parent.display());
            }
        }
    }

    let file_exists = shadow_path.exists();

    if !file_exists {
        match create_shadow_file(shadow_path) {
            Ok(()) => log::info!("Created shadow file {} (mode 600)", shadow.path),
            Err(e) => log::error!("Failed to create shadow file {}: {e}", shadow.path),
        }
    }

    let mut auth_config = match rshs::AuthConfig::load_from_shadow_file(shadow_path) {
        Ok(cfg) => {
            if cfg.user_count() > 0 {
                log::info!(
                    "Loaded {} users from shadow file {}",
                    cfg.user_count(),
                    shadow.path
                );
            }
            cfg
        }
        Err(e) => {
            log::error!("{e}");
            rshs::AuthConfig::new()
        }
    };

    if !cli_auth.is_empty() {
        auth_config.merge_cli(&cli_auth);
    }

    if cli.shadow_write && shadow.writable {
        match auth_config.write_to_shadow_file(shadow_path, false) {
            Ok(()) => {
                log::info!(
                    "Wrote {} users to shadow file {}",
                    auth_config.user_count(),
                    shadow.path
                );
            }
            Err(e) => {
                log::error!("Failed to write shadow file: {e}");
            }
        }
    }

    auth_config
}

fn create_shadow_file(path: &Path) -> std::io::Result<()> {
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}
