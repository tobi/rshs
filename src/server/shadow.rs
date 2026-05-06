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
        tracing::warn!(
            path = %shadow.path,
            "shadow file is read-only (:ro), ignoring --shadow-write"
        );
    }

    if let Some(parent) = shadow_path.parent() {
        let parent_str = parent.as_os_str();
        if !parent_str.is_empty() && !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::error!(path = %parent.display(), error = %e, "Failed to create directory");
            } else {
                tracing::info!(path = %parent.display(), "Created directory");
            }
        }
    }

    let file_exists = shadow_path.exists();

    if !file_exists {
        match create_shadow_file(shadow_path) {
            Ok(()) => tracing::info!(path = %shadow.path, "Created shadow file (mode 600)"),
            Err(e) => {
                tracing::error!(path = %shadow.path, error = %e, "Failed to create shadow file")
            }
        }
    }

    let mut auth_config = match AuthConfig::load_from_shadow_file(shadow_path) {
        Ok(cfg) => {
            if cfg.user_count() > 0 {
                tracing::info!(
                    count = cfg.user_count(),
                    path = %shadow.path,
                    "Loaded users from shadow file"
                );
            }
            cfg
        }
        Err(e) => {
            tracing::error!(error = %e, path = %shadow.path, "Failed to load shadow file");
            AuthConfig::new()
        }
    };

    if !cli_auth.is_empty() {
        auth_config.merge_cli(&cli_auth);
    }

    if cli.shadow_write {
        if !is_path_writable(shadow_path) {
            tracing::warn!(
                path = %shadow.path,
                "shadow file is read-only (OS), ignoring --shadow-write"
            );
        } else {
            match auth_config.write_to_shadow_file(shadow_path, false) {
                Ok(()) => {
                    tracing::info!(
                        count = auth_config.user_count(),
                        path = %shadow.path,
                        "Wrote users to shadow file"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to write shadow file");
                }
            }
        }
    } else if shadow.writable && shadow_path.exists() && !is_path_writable(shadow_path) {
        tracing::warn!(
            path = %shadow.path,
            "shadow file is declared :rw but file is read-only at OS level"
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
