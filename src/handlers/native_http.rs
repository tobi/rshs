use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::StreamReader;

use crate::server::AppState;
use crate::utils::path;

pub async fn handle_put(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = match path::resolve_write_target(&state.root_dir, &request_path) {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Create parent directories
    let parent = fs_path.parent().unwrap_or(&state.root_dir);
    if let Err(e) = tokio::fs::create_dir_all(parent).await {
        tracing::error!(
            error = %e, path = %parent.display(), "failed to create parent directories for PUT"
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Canonicalize parent to detect symlink escapes
    let parent_canonical = match tokio::fs::canonicalize(parent).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(
                error = %e, path = %parent.display(), "failed to canonicalize parent for PUT"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !parent_canonical.starts_with(state.root_canonical.as_path()) {
        tracing::warn!(path = %fs_path.display(), "path traversal blocked in PUT");
        return StatusCode::FORBIDDEN.into_response();
    }

    let filename = fs_path.file_name().unwrap();
    let target = parent_canonical.join(filename);

    let existed = match tokio::fs::metadata(&target).await {
        Ok(m) => m.is_file(),
        Err(_) => false,
    };

    // Stream body to file via StreamReader + io::copy
    let mut file = match tokio::fs::File::create(&target).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "failed to create file for PUT"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let body = req.into_body();
    let stream = body.into_data_stream().map_err(std::io::Error::other);
    let mut reader = StreamReader::new(stream);

    let bytes_written = match tokio::io::copy(&mut reader, &mut file).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "error writing PUT body"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = file.flush().await {
        tracing::error!(error = %e, path = %target.display(), "error flushing PUT file");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    tracing::debug!(
        path = %target.display(), size = bytes_written, existed = existed, "PUT completed"
    );

    if existed {
        StatusCode::OK.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

pub async fn handle_options() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "allow",
            "GET, HEAD, OPTIONS, PUT, DELETE, PROPFIND, MKCOL, COPY, MOVE, PROPPATCH, LOCK, UNLOCK",
        )
        .header("content-length", "0")
        .body(Body::empty())
        .unwrap()
}

pub async fn handle_delete(State(state): State<Arc<AppState>>, req: Request) -> Response {
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

    use crate::{AppState, AuthConfig, handlers::native_http};

    fn make_app(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let put_handler = axum::routing::put(native_http::handle_put);
        Router::new()
            .route("/", put_handler.clone().delete(native_http::handle_delete))
            .route(
                "/{*path}",
                axum::routing::put(native_http::handle_put).delete(native_http::handle_delete),
            )
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::webdav::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
            }))
    }

    #[tokio::test]
    async fn test_put_creates_new_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/newfile.txt")
            .header("content-type", "text/plain")
            .body(Body::from("hello put"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);

        let content = std::fs::read_to_string(dir.path().join("newfile.txt")).unwrap();
        assert_eq!(content, "hello put");
    }

    #[tokio::test]
    async fn test_put_overwrites_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("existing.txt"), b"old content").unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/existing.txt")
            .body(Body::from("new content"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let content = std::fs::read_to_string(dir.path().join("existing.txt")).unwrap();
        assert_eq!(content, "new content");
    }

    #[tokio::test]
    async fn test_put_empty_body() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/empty.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);

        let content = std::fs::read_to_string(dir.path().join("empty.txt")).unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_put_creates_parent_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/a/b/c/file.txt")
            .body(Body::from("nested"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);

        let content = std::fs::read_to_string(dir.path().join("a/b/c/file.txt")).unwrap();
        assert_eq!(content, "nested");
    }

    #[tokio::test]
    async fn test_put_rejects_root_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_put_rejects_dir_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/subdir/")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_put_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/../outside.txt")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
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

    #[tokio::test]
    async fn test_options_returns_ok() {
        let resp = native_http::handle_options().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let allow = resp.headers().get("allow").unwrap().to_str().unwrap();
        assert!(allow.contains("GET"));
        assert!(allow.contains("PUT"));
        assert!(allow.contains("DELETE"));
        assert!(allow.contains("PROPFIND"));
        assert!(allow.contains("MKCOL"));
    }

    #[tokio::test]
    async fn test_options_has_content_length_zero() {
        let resp = native_http::handle_options().await;
        let cl = resp
            .headers()
            .get("content-length")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cl, "0");
    }

    #[tokio::test]
    async fn test_options_body_empty() {
        let resp = native_http::handle_options().await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());
    }
}
