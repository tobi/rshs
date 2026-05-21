use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use derive_new::new;
use sha_crypt::{PasswordHasher, PasswordVerifier, ShaCrypt};

use crate::cli::Cli;

#[derive(Debug, Clone)]
pub enum Credential {
    Plaintext(String),
    Sha512Crypt(String),
}

#[derive(Debug, Clone, new)]
pub struct ShadowFileArg {
    pub path: String,
    pub writable: bool,
}

impl ShadowFileArg {
    /// Parse a shadow file spec string: `PATH[:rw|:ro]`. Defaults to `:rw`.
    pub fn from_arg(s: &str) -> Self {
        if let Some(path) = s.strip_suffix(":rw") {
            Self::new(path.to_string(), true)
        } else if let Some(path) = s.strip_suffix(":ro") {
            Self::new(path.to_string(), false)
        } else {
            Self::new(s.to_string(), true)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub users: HashMap<String, Credential>,
}

impl AuthConfig {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    pub fn add_user(&mut self, username: &str, password: &str) {
        self.users.insert(
            username.to_string(),
            Credential::Plaintext(password.to_string()),
        );
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    pub fn validate(&self, username: &str, password: &str) -> bool {
        match self.users.get(username) {
            Some(Credential::Plaintext(expected)) => expected == password,
            Some(Credential::Sha512Crypt(hash)) => ShaCrypt::default()
                .verify_password(password.as_bytes(), hash.as_str())
                .is_ok(),
            None => false,
        }
    }

    pub fn merge_cli(&mut self, other: &AuthConfig) {
        for (username, credential) in &other.users {
            self.users.insert(username.clone(), credential.clone());
        }
    }

    pub fn load_from_shadow_file(path: &Path) -> io::Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot read shadow file {}: {e}", path.display()),
            )
        })?;
        let mut config = AuthConfig::new();

        for (line_no, line) in content.lines().enumerate() {
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            match line.split_once(':') {
                Some((username, rest)) if !username.is_empty() => {
                    let hash = if let Some((h, _)) = rest.split_once(':') {
                        h
                    } else {
                        rest
                    };

                    if hash.is_empty() || !hash.starts_with('$') {
                        tracing::warn!(
                            path = %path.display(), line = line_no + 1, "unsupported hash format, skipping"
                        );
                        continue;
                    }

                    config.users.insert(
                        username.to_string(),
                        Credential::Sha512Crypt(hash.to_string()),
                    );
                }
                _ => {
                    tracing::warn!(
                        path = %path.display(), line = line_no + 1, "malformed entry, skipping"
                    );
                }
            }
        }

        Ok(config)
    }

    pub fn write_to_shadow_file(&self, path: &Path, create: bool) -> io::Result<()> {
        if create {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("cannot create parent dir {}: {e}", parent.display()),
                    )
                })?;
            }

            let file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("cannot create shadow file {}: {e}", path.display()),
                    )
                })?;

            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("cannot set permissions on {}: {e}", path.display()),
                    )
                })?;
        }

        let mut content = String::new();
        for (username, credential) in &self.users {
            let hash = match credential {
                Credential::Sha512Crypt(h) => h.clone(),
                Credential::Plaintext(p) => ShaCrypt::default()
                    .hash_password(p.as_bytes())
                    .map_err(|e| io::Error::other(format!("cannot hash password: {e}")))?
                    .to_string(),
            };
            content.push_str(username);
            content.push(':');
            content.push_str(&hash);
            content.push('\n');
        }

        fs::write(path, &content).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot write shadow file {}: {e}", path.display()),
            )
        })?;

        Ok(())
    }
}

pub fn build_auth_config(cli: &Cli) -> AuthConfig {
    let cli_auth = cli.to_auth_config();

    let Some(shadow) = cli.to_shadow_file_arg() else {
        return cli_auth;
    };

    let shadow_path = Path::new(&shadow.path);

    if cli.shadow_write && !shadow.writable {
        tracing::warn!(
            path = %shadow.path, "shadow file is read-only (:ro), ignoring --shadow-write"
        );
    }

    if let Some(parent) = shadow_path.parent() {
        let parent_str = parent.as_os_str();
        if !parent_str.is_empty() && !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::error!(path = %parent.display(), error = %e, "failed to create directory");
            } else {
                tracing::info!(path = %parent.display(), "created directory");
            }
        }
    }

    let file_exists = shadow_path.exists();

    if !file_exists {
        match create_shadow_file(shadow_path) {
            Ok(()) => tracing::info!(path = %shadow.path, "created shadow file (mode 600)"),
            Err(e) => {
                tracing::error!(path = %shadow.path, error = %e, "failed to create shadow file")
            }
        }
    }

    let mut auth_config = match AuthConfig::load_from_shadow_file(shadow_path) {
        Ok(cfg) => {
            if cfg.user_count() > 0 {
                tracing::info!(
                    count = cfg.user_count(), path = %shadow.path, "loaded users from shadow file"
                );
            }
            cfg
        }
        Err(e) => {
            tracing::error!(error = %e, path = %shadow.path, "failed to load shadow file");
            AuthConfig::new()
        }
    };

    if !cli_auth.is_empty() {
        auth_config.merge_cli(&cli_auth);
    }

    if cli.shadow_write {
        if !is_path_writable(shadow_path) {
            tracing::warn!(
                path = %shadow.path, "shadow file is read-only (OS), ignoring --shadow-write"
            );
        } else {
            match auth_config.write_to_shadow_file(shadow_path, false) {
                Ok(()) => {
                    tracing::info!(
                        count = auth_config.user_count(), path = %shadow.path, "wrote users to shadow file"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to write shadow file");
                }
            }
        }
    } else if shadow.writable && shadow_path.exists() && !is_path_writable(shadow_path) {
        tracing::warn!(
            path = %shadow.path, "shadow file is declared :rw but file is read-only at OS level"
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

    fn make_hashed_entry(username: &str, password: &str) -> String {
        let hash = ShaCrypt::default()
            .hash_password(password.as_bytes())
            .unwrap()
            .to_string();
        format!("{username}:{hash}\n")
    }

    #[test]
    fn test_validate_plaintext() {
        let mut config = AuthConfig::new();
        config.add_user("admin", "secret");
        assert!(config.validate("admin", "secret"));
        assert!(!config.validate("admin", "wrong"));
        assert!(!config.validate("nobody", "secret"));
    }

    #[test]
    fn test_validate_sha512_crypt() {
        let hash = ShaCrypt::default()
            .hash_password("mypassword".as_bytes())
            .unwrap()
            .to_string();
        let mut config = AuthConfig::new();
        config
            .users
            .insert("admin".to_string(), Credential::Sha512Crypt(hash));
        assert!(config.validate("admin", "mypassword"));
        assert!(!config.validate("admin", "wrong"));
    }

    #[test]
    fn test_validate_mixed_credentials() {
        let hash = ShaCrypt::default()
            .hash_password("hashedpass".as_bytes())
            .unwrap()
            .to_string();
        let mut config = AuthConfig::new();
        config.add_user("cli_user", "plainpass");
        config
            .users
            .insert("file_user".to_string(), Credential::Sha512Crypt(hash));
        assert!(config.validate("cli_user", "plainpass"));
        assert!(!config.validate("cli_user", "wrong"));
        assert!(config.validate("file_user", "hashedpass"));
        assert!(!config.validate("file_user", "wrong"));
    }

    #[test]
    fn test_load_shadow_file_single_user() {
        let content = make_hashed_entry("admin", "adminpass");
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &content).unwrap();

        let config = AuthConfig::load_from_shadow_file(file.path()).unwrap();
        assert!(!config.is_empty());
        assert_eq!(config.user_count(), 1);
        assert!(config.validate("admin", "adminpass"));
        assert!(!config.validate("admin", "wrong"));
    }

    #[test]
    fn test_load_shadow_file_multiple_users() {
        let mut content = make_hashed_entry("alice", "alicepass");
        content.push_str(&make_hashed_entry("bob", "bobpass"));
        content.push_str(&make_hashed_entry("carol", "carolpass"));
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &content).unwrap();

        let config = AuthConfig::load_from_shadow_file(file.path()).unwrap();
        assert_eq!(config.user_count(), 3);
        assert!(config.validate("alice", "alicepass"));
        assert!(config.validate("bob", "bobpass"));
        assert!(config.validate("carol", "carolpass"));
    }

    #[test]
    fn test_load_shadow_file_skips_empty_lines() {
        let mut content = String::new();
        content.push('\n');
        content.push_str(&make_hashed_entry("admin", "adminpass"));
        content.push_str("\n\n");
        content.push_str(&make_hashed_entry("viewer", "viewerpass"));
        content.push('\n');
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &content).unwrap();

        let config = AuthConfig::load_from_shadow_file(file.path()).unwrap();
        assert_eq!(config.user_count(), 2);
        assert!(config.validate("admin", "adminpass"));
        assert!(config.validate("viewer", "viewerpass"));
    }

    #[test]
    fn test_load_shadow_file_skips_malformed_lines() {
        let mut content = String::new();
        content.push_str(&make_hashed_entry("admin", "adminpass"));
        content.push_str("malformed_line_without_colon\n");
        content.push_str(":nousername\n");
        content.push_str(&make_hashed_entry("viewer", "viewerpass"));
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &content).unwrap();

        let config = AuthConfig::load_from_shadow_file(file.path()).unwrap();
        assert_eq!(config.user_count(), 2);
        assert!(config.validate("admin", "adminpass"));
        assert!(config.validate("viewer", "viewerpass"));
    }

    #[test]
    fn test_load_shadow_file_nonexistent() {
        let result = AuthConfig::load_from_shadow_file(Path::new("/nonexistent/shadow/file"));
        assert!(result.is_err());
    }

    #[test]
    fn test_merge_cli_overwrites() {
        let mut base = AuthConfig::new();
        base.add_user("admin", "shadow_pass");
        let mut cli_auth = AuthConfig::new();
        cli_auth.add_user("admin", "cli_pass");
        cli_auth.add_user("viewer", "viewer_pass");

        base.merge_cli(&cli_auth);

        assert_eq!(base.user_count(), 2);
        assert!(base.validate("admin", "cli_pass"));
        assert!(!base.validate("admin", "shadow_pass"));
        assert!(base.validate("viewer", "viewer_pass"));
    }

    #[test]
    fn test_merge_cli_adds_new_users() {
        let mut base = AuthConfig::new();
        base.add_user("existing", "existing_pass");
        let mut cli_auth = AuthConfig::new();
        cli_auth.add_user("new_user", "new_pass");

        base.merge_cli(&cli_auth);

        assert_eq!(base.user_count(), 2);
        assert!(base.validate("existing", "existing_pass"));
        assert!(base.validate("new_user", "new_pass"));
    }

    #[test]
    fn test_write_to_shadow_file_roundtrip() {
        let mut config = AuthConfig::new();
        config.add_user("admin", "adminpass");
        config.add_user("viewer", "viewerpass");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shadow");

        config.write_to_shadow_file(&path, true).unwrap();

        let loaded = AuthConfig::load_from_shadow_file(&path).unwrap();
        assert_eq!(loaded.user_count(), 2);
        assert!(loaded.validate("admin", "adminpass"));
        assert!(loaded.validate("viewer", "viewerpass"));
        assert!(!loaded.validate("admin", "wrong"));
    }

    #[test]
    fn test_write_to_shadow_file_hashes_plaintext() {
        let mut config = AuthConfig::new();
        config.add_user("admin", "secret123");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shadow");

        config.write_to_shadow_file(&path, true).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("admin:$6$"));

        let loaded = AuthConfig::load_from_shadow_file(&path).unwrap();
        assert!(loaded.validate("admin", "secret123"));
        assert!(!loaded.validate("admin", "wrong"));
    }

    #[test]
    fn test_create_file_with_mode_600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shadow");

        let mut config = AuthConfig::new();
        config.add_user("admin", "secret");

        config.write_to_shadow_file(&path, true).unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn test_is_empty_and_user_count() {
        let mut config = AuthConfig::new();
        assert!(config.is_empty());
        assert_eq!(config.user_count(), 0);

        config.add_user("admin", "secret");
        assert!(!config.is_empty());
        assert_eq!(config.user_count(), 1);
    }

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
