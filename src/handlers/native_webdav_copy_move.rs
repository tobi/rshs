use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::server::AppState;
use crate::utils::path;
use crate::webdav;

pub async fn handle_copy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    do_move_or_copy(&state, req, false).await
}

pub async fn handle_move(State(state): State<Arc<AppState>>, req: Request) -> Response {
    do_move_or_copy(&state, req, true).await
}

async fn do_move_or_copy(state: &Arc<AppState>, req: Request, is_move: bool) -> Response {
    let verb = if is_move { "MOVE" } else { "COPY" };
    let headers = req.headers();
    let overwrite = webdav::parse_overwrite(headers);

    let dest_str = match webdav::parse_destination(headers) {
        Some(s) => s,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };
    let src_path = req.uri().path().to_owned();

    let fs_src =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &src_path).await {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND.into_response(),
        };

    let fs_dest = match path::resolve_write_target(&state.root_dir, &dest_str) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN.into_response(),
    };

    if fs_src == fs_dest {
        return StatusCode::FORBIDDEN.into_response();
    }

    let dest_parent = fs_dest.parent().unwrap_or(&state.root_dir);
    if let Err(e) = tokio::fs::create_dir_all(dest_parent).await {
        tracing::error!(error = %e, path = %dest_parent.display(), "failed to create dest parent dirs");
        return StatusCode::CONFLICT.into_response();
    }

    let parent_canonical = match tokio::fs::canonicalize(dest_parent).await {
        Ok(p) => p,
        Err(_) => return StatusCode::CONFLICT.into_response(),
    };
    if !parent_canonical.starts_with(state.root_canonical.as_path()) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let filename = fs_dest.file_name().unwrap();
    let dest = parent_canonical.join(filename);
    let dest_existed = tokio::fs::metadata(&dest).await.is_ok();

    if dest_existed && !overwrite {
        tracing::debug!(verb, "target exists and Overwrite is F");
        return StatusCode::PRECONDITION_FAILED.into_response();
    }

    let meta = match tokio::fs::metadata(&fs_src).await {
        Ok(m) => m,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    if meta.is_dir() {
        if let Err(resp) = copy_dir(&fs_src, &dest, dest_existed).await {
            return resp;
        }
    } else if let Err(resp) = copy_file(&fs_src, &dest).await {
        return resp;
    }

    if is_move && tokio::fs::rename(&fs_src, &dest).await.is_err() {
        if meta.is_dir() {
            let _ = tokio::fs::remove_dir_all(&fs_src).await;
        } else {
            let _ = tokio::fs::remove_file(&fs_src).await;
        }
    }

    tracing::debug!(verb, src = %fs_src.display(), dest = %dest.display(), "completed");
    if dest_existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

async fn copy_file(src: &std::path::Path, dest: &std::path::Path) -> Result<(), Response> {
    tokio::fs::copy(src, dest).await.map_err(|e| {
        tracing::error!(error = %e, src = %src.display(), dest = %dest.display(), "copy file failed");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;
    Ok(())
}

async fn copy_dir(
    src: &std::path::Path,
    dest: &std::path::Path,
    dest_existed: bool,
) -> Result<(), Response> {
    if !dest_existed {
        tokio::fs::create_dir(dest).await.map_err(|e| {
            tracing::error!(error = %e, dest = %dest.display(), "create dest dir failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    }

    let mut stack = vec![(src.to_path_buf(), dest.to_path_buf())];
    while let Some((src_dir, dest_dir)) = stack.pop() {
        let mut read_dir = tokio::fs::read_dir(&src_dir).await.map_err(|e| {
            tracing::error!(error = %e, dir = %src_dir.display(), "read dir failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            tracing::error!(error = %e, "read entry failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })? {
            let file_type = entry.file_type().await.map_err(|e| {
                tracing::error!(error = %e, "file_type failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            })?;
            let entry_dest = dest_dir.join(entry.file_name());

            if file_type.is_dir() {
                tokio::fs::create_dir(&entry_dest).await.map_err(|e| {
                    tracing::error!(error = %e, dest = %entry_dest.display(), "create sub dir failed");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                })?;
                stack.push((entry.path(), entry_dest));
            } else if file_type.is_symlink() {
                continue;
            } else {
                tokio::fs::copy(entry.path(), &entry_dest).await.map_err(|e| {
                    tracing::error!(error = %e, src = %entry.path().display(), dest = %entry_dest.display(), "copy file failed");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request, routing::any};
    use tower::ServiceExt;

    use crate::{AppState, AuthConfig};

    // -- COPY tests ---------------------------------------------------------

    fn make_app_copy(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle_copy))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            }))
    }

    fn make_copy(uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .method(axum::http::Method::from_bytes(b"COPY").unwrap())
            .uri(uri)
            .header("destination", dest);
        if let Some(ov) = overwrite {
            builder = builder.header("overwrite", ov);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn test_copy_file_creates() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"hello").unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/s.txt", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("d.txt")).unwrap(),
            "hello"
        );
        assert!(dir.path().join("s.txt").exists());
    }

    #[tokio::test]
    async fn test_copy_file_overwrite() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"new").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"old").unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/s.txt", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("d.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn test_copy_overwrite_false() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"b").unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/s.txt", "http://x/d.txt", Some("F"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_copy_source_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/ghost", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_copy_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sd")).unwrap();
        std::fs::write(dir.path().join("sd/a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("sd/b.txt"), b"b").unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/sd", "http://x/dd", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert!(dir.path().join("dd").is_dir());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dd/a.txt")).unwrap(),
            "a"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dd/b.txt")).unwrap(),
            "b"
        );
    }

    #[tokio::test]
    async fn test_copy_no_dest_header() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"x").unwrap();
        let app = make_app_copy(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"COPY").unwrap())
            .uri("/s.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    // -- MOVE tests ---------------------------------------------------------

    fn make_app_move(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle_move))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            }))
    }

    fn make_move(uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .method(axum::http::Method::from_bytes(b"MOVE").unwrap())
            .uri(uri)
            .header("destination", dest);
        if let Some(ov) = overwrite {
            builder = builder.header("overwrite", ov);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn test_move_file_creates() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"hello").unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/s.txt", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("d.txt")).unwrap(),
            "hello"
        );
        assert!(!dir.path().join("s.txt").exists());
    }

    #[tokio::test]
    async fn test_move_file_overwrite() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"new").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"old").unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/s.txt", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("d.txt")).unwrap(),
            "new"
        );
        assert!(!dir.path().join("s.txt").exists());
    }

    #[tokio::test]
    async fn test_move_overwrite_false() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"b").unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/s.txt", "http://x/d.txt", Some("F"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_move_source_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/ghost", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_move_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sd")).unwrap();
        std::fs::write(dir.path().join("sd/a.txt"), b"a").unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/sd", "http://x/dd", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert!(dir.path().join("dd").is_dir());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dd/a.txt")).unwrap(),
            "a"
        );
        assert!(!dir.path().join("sd").exists());
    }

    #[tokio::test]
    async fn test_move_no_dest_header() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"x").unwrap();
        let app = make_app_move(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"MOVE").unwrap())
            .uri("/s.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
