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

    let shadow_path = Path::new(&shadow.path);

    if cli.shadow_write && !shadow.writable {
        log::warn!(
            "shadow file {} is read-only (ro:), ignoring --shadow-write",
            shadow.path
        );
    }

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

    if cli.shadow_write {
        if !is_path_writable(shadow_path) {
            log::warn!(
                "shadow file {} is read-only (OS), ignoring --shadow-write",
                shadow.path
            );
        } else {
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
    } else if shadow.writable && shadow_path.exists() && !is_path_writable(shadow_path) {
        log::warn!(
            "shadow file {} is declared rw: but file is read-only at OS level",
            shadow.path
        );
    }

    auth_config
}

fn is_path_writable(path: &Path) -> bool {
    if let Ok(meta) = fs::metadata(path) {
        !meta.permissions().readonly()
    } else if let Some(parent) = path.parent() {
        fs::metadata(parent)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(true)
    } else {
        true
    }
}

fn create_shadow_file(path: &Path) -> std::io::Result<()> {
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_is_path_writable_existing_file() {
        let file = NamedTempFile::new().unwrap();
        assert!(is_path_writable(file.path()));
    }

    #[test]
    fn test_is_path_writable_readonly_file() {
        let file = NamedTempFile::new().unwrap();
        let mut perms = fs::metadata(file.path()).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(file.path(), perms).unwrap();
        assert!(!is_path_writable(file.path()));
    }

    #[test]
    fn test_is_path_writable_nonexistent_in_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent");
        assert!(is_path_writable(&path));
    }
}
