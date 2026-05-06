use actix_web::web;
use actix_web_httpauth::extractors::basic::BasicAuth;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Credential {
    Plaintext(String),
    Sha512Crypt(String),
}

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub(crate) users: HashMap<String, Credential>,
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
            Some(Credential::Sha512Crypt(hash)) => sha_crypt::sha512_check(password, hash).is_ok(),
            None => false,
        }
    }

    pub fn merge_cli(&mut self, other: &AuthConfig) {
        for (username, credential) in &other.users {
            self.users.insert(username.clone(), credential.clone());
        }
    }

    pub fn load_from_shadow_file(path: &Path) -> Result<Self, String> {
        let file = fs::File::open(path)
            .map_err(|e| format!("cannot open shadow file {}: {e}", path.display()))?;
        let reader = BufReader::new(file);
        let mut config = AuthConfig::new();

        for (line_no, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("cannot read shadow file: {e}"))?;
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
                            path = %path.display(),
                            line = line_no + 1,
                            "unsupported hash format, skipping"
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
                        path = %path.display(),
                        line = line_no + 1,
                        "malformed entry, skipping"
                    );
                }
            }
        }

        Ok(config)
    }

    pub fn write_to_shadow_file(&self, path: &Path, create: bool) -> Result<(), String> {
        if create {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create parent dir {}: {e}", parent.display()))?;
            }

            let file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| format!("cannot create shadow file {}: {e}", path.display()))?;

            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("cannot set permissions on {}: {e}", path.display()))?;
        }

        let mut content = String::new();
        for (username, credential) in &self.users {
            let hash = match credential {
                Credential::Sha512Crypt(h) => h.clone(),
                Credential::Plaintext(p) => {
                    sha_crypt::sha512_simple(p, &sha_crypt::Sha512Params::default())
                        .map_err(|e| format!("cannot hash password: {e:?}"))?
                }
            };
            content.push_str(username);
            content.push(':');
            content.push_str(&hash);
            content.push('\n');
        }

        fs::write(path, &content)
            .map_err(|e| format!("cannot write shadow file {}: {e}", path.display()))?;

        Ok(())
    }
}

pub async fn auth_validator(
    req: actix_web::dev::ServiceRequest,
    credentials: BasicAuth,
) -> Result<actix_web::dev::ServiceRequest, (actix_web::Error, actix_web::dev::ServiceRequest)> {
    let config = req
        .app_data::<web::Data<AuthConfig>>()
        .expect("AuthConfig not found in app data");

    let password = credentials.password().unwrap_or("");
    let username = credentials.user_id();

    if config.validate(username, password) {
        tracing::debug!(user = username, outcome = "success");
        Ok(req)
    } else {
        tracing::warn!(
            user = username,
            peer = %req.connection_info()
                .peer_addr()
                .unwrap_or("unknown"),
            outcome = "failure",
        );
        let error = actix_web::error::ErrorUnauthorized(r#"Basic realm="rshs""#);
        Err((error, req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_hashed_entry(username: &str, password: &str) -> String {
        let hash = sha_crypt::sha512_simple(password, &sha_crypt::Sha512Params::default()).unwrap();
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
        let hash =
            sha_crypt::sha512_simple("mypassword", &sha_crypt::Sha512Params::default()).unwrap();
        let mut config = AuthConfig::new();
        config
            .users
            .insert("admin".to_string(), Credential::Sha512Crypt(hash));
        assert!(config.validate("admin", "mypassword"));
        assert!(!config.validate("admin", "wrong"));
    }

    #[test]
    fn test_validate_mixed_credentials() {
        let hash =
            sha_crypt::sha512_simple("hashedpass", &sha_crypt::Sha512Params::default()).unwrap();
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
}
