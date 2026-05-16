use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::StreamReader;

use crate::server::AppState;
use crate::utils::path;

pub async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
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
            .route("/", axum::routing::put(super::handle))
            .route("/{*path}", axum::routing::put(super::handle))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
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
}
