use crate::cli::Cli;
use crate::server::auth_basic::AuthConfig;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn build_auth_config(cli: &Cli) -> AuthConfig {
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

    let mut auth_config = match AuthConfig::load_from_shadow_file(shadow_path) {
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
            AuthConfig::new()
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
