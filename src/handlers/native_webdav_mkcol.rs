use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::server::AppState;
use crate::utils::path;

pub async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = match path::resolve_write_target(&state.root_dir, &request_path) {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed for MKCOL");
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    let parent = fs_path.parent().unwrap_or(&state.root_dir);
    let parent_canonical = match tokio::fs::canonicalize(parent).await {
        Ok(p) => p,
        Err(_) => {
            tracing::debug!("parent not found for MKCOL");
            return StatusCode::CONFLICT.into_response();
        }
    };

    if !parent_canonical.starts_with(state.root_canonical.as_path()) {
        tracing::warn!(path = %fs_path.display(), "path traversal blocked in MKCOL");
        return StatusCode::FORBIDDEN.into_response();
    }

    let filename = fs_path.file_name().unwrap();
    let target = parent_canonical.join(filename);

    if tokio::fs::metadata(&target).await.is_ok() {
        tracing::debug!(path = %target.display(), "MKCOL target already exists");
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    match tokio::fs::create_dir(&target).await {
        Ok(()) => {
            tracing::debug!(path = %target.display(), "MKCOL completed");
            StatusCode::CREATED.into_response()
        }
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "failed to create directory for MKCOL"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request, routing::any};
    use tower::ServiceExt;

    use crate::{AppState, AuthConfig};

    fn make_app(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
                locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
            }))
    }

    fn make_mkcol(uri: &str) -> Request {
        Request::builder()
            .method(axum::http::Method::from_bytes(b"MKCOL").unwrap())
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_mkcol_creates_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/newdir");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert!(dir.path().join("newdir").is_dir());
    }

    #[tokio::test]
    async fn test_mkcol_parent_not_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/no_parent/newdir");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/d");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/f.txt");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_mkcol_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = make_mkcol("/../outside");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }
}
