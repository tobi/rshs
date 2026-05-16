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

    let fs_path =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &request_path).await {
            Some(p) => p,
            None => {
                tracing::debug!("path resolution failed for DELETE");
                return StatusCode::NOT_FOUND.into_response();
            }
        };

    let meta = match tokio::fs::metadata(&fs_path).await {
        Ok(m) => m,
        Err(_) => {
            tracing::debug!("metadata failed for DELETE");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    if meta.is_dir() {
        if fs_path == state.root_canonical {
            tracing::debug!("DELETE rejected: root directory");
            return StatusCode::BAD_REQUEST.into_response();
        }
        match tokio::fs::remove_dir_all(&fs_path).await {
            Ok(()) => {
                tracing::debug!(path = %fs_path.display(), "DELETE directory completed");
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => {
                tracing::error!(
                    error = %e, path = %fs_path.display(), "failed to remove directory for DELETE"
                );
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    } else {
        match tokio::fs::remove_file(&fs_path).await {
            Ok(()) => {
                tracing::debug!(path = %fs_path.display(), "DELETE completed");
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => {
                tracing::error!(
                    error = %e, path = %fs_path.display(), "failed to remove file for DELETE"
                );
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request};
    use tower::ServiceExt;

    use crate::{AppState, AuthConfig};

    fn make_app(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .route("/", axum::routing::delete(super::handle))
            .route("/{*path}", axum::routing::delete(super::handle))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
                locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
            }))
    }

    #[tokio::test]
    async fn test_delete_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("remove_me.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/remove_me.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!dir.path().join("remove_me.txt").exists());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/ghost.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("mydir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("inner.txt"), b"inside").unwrap();
        assert!(subdir.exists());
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/mydir")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!subdir.exists());
    }

    #[tokio::test]
    async fn test_delete_root_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_delete_dir_trailing_slash() {
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("mydir");
        std::fs::create_dir(&subdir).unwrap();
        assert!(subdir.exists());
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/mydir/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!subdir.exists());
    }

    #[tokio::test]
    async fn test_delete_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/../outside.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
