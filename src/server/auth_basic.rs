use std::collections::HashMap;

use actix_web::web;
use actix_web_httpauth::extractors::basic::BasicAuth;

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub(crate) users: HashMap<String, String>,
}

impl AuthConfig {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    pub fn add_user(&mut self, username: &str, password: &str) {
        self.users
            .insert(username.to_string(), password.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    pub fn validate(&self, username: &str, password: &str) -> bool {
        self.users
            .get(username)
            .is_some_and(|expected| expected == password)
    }

    pub fn merge(&mut self, other: &AuthConfig) {
        for (user, pass) in &other.users {
            self.users
                .entry(user.clone())
                .or_insert_with(|| pass.clone());
        }
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

    if config.validate(credentials.user_id(), password) {
        Ok(req)
    } else {
        let error = actix_web::error::ErrorUnauthorized(r#"Basic realm="rshs""#);
        Err((error, req))
    }
}
