use std::sync::Arc;

use axum::{
    body::{self, Body},
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::server::AppState;
use crate::utils::path;
use crate::webdav;

pub async fn handle_propfind(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let depth = webdav::parse_depth(req.headers());
    let request_path = req.uri().path().to_owned();

    let fs_path =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &request_path).await {
            Some(p) => p,
            None => {
                tracing::debug!("path resolution failed for PROPFIND");
                return StatusCode::NOT_FOUND.into_response();
            }
        };

    let body_bytes = match body::to_bytes(req.into_body(), 65536).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "failed to read PROPFIND body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let prop_request = match webdav::parse_propfind_request(&body_bytes) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "failed to parse PROPFIND request");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let entries = webdav::fs::collect_entries(&fs_path, &request_path, depth).await;
    let xml = webdav::xml::build_multistatus(&entries, &prop_request);

    tracing::debug!(
        path = %fs_path.display(), depth = ?depth, entries = entries.len(), "PROPFIND completed"
    );

    Response::builder()
        .status(StatusCode::from_u16(207).unwrap())
        .header("content-type", "application/xml; charset=utf-8")
        .body(Body::from(xml))
        .unwrap()
}

pub async fn handle_mkcol(State(state): State<Arc<AppState>>, req: Request) -> Response {
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

pub async fn handle_copy(State(state): State<Arc<AppState>>, req: Request) -> Response {
    do_move_or_copy(&state, req, false).await
}

pub async fn handle_move(State(state): State<Arc<AppState>>, req: Request) -> Response {
    do_move_or_copy(&state, req, true).await
}

async fn do_move_or_copy(state: &Arc<AppState>, req: Request, is_move: bool) -> Response {
    let verb = if is_move { "MOVE" } else { "COPY" }; // must be before req.headers()
    let headers = req.headers();
    let overwrite = webdav::parse_overwrite(headers);

    // Inline resolve_dest to avoid lifetime issues
    let headers = req.headers();
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
                // Skip symlinks
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

    fn make_app(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle_propfind))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
            }))
    }

    fn propfind_body(props: &str) -> Body {
        Body::from(format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:propfind xmlns:D=\"DAV:\"><D:prop>{}</D:prop></D:propfind>",
            props
        ))
    }

    fn make_propfind(uri: &str, depth: &str, body: Body) -> Request {
        Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPFIND").unwrap())
            .uri(uri)
            .header("depth", depth)
            .body(body)
            .unwrap()
    }

    #[tokio::test]
    async fn test_propfind_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let app = make_app(&dir);

        let req = make_propfind(
            "/f.txt",
            "0",
            propfind_body("<D:getcontentlength/><D:getlastmodified/><D:resourcetype/>"),
        );
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("multistatus"));
        assert!(text.contains("/f.txt"));
        assert!(text.contains("getcontentlength"));
        assert!(text.contains("5"));
        assert!(!text.contains("collection"));
    }

    #[tokio::test]
    async fn test_propfind_dir_depth0() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        let app = make_app(&dir);

        let req = make_propfind("/d", "0", propfind_body("<D:resourcetype/>"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("collection"));
        assert!(!text.contains("getcontentlength"));
    }

    #[tokio::test]
    async fn test_propfind_dir_depth1() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        std::fs::write(dir.path().join("d/a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d/b.txt"), b"bb").unwrap();
        let app = make_app(&dir);

        let req = make_propfind(
            "/d",
            "1",
            propfind_body("<D:getcontentlength/><D:resourcetype/>"),
        );
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(text.matches("<D:response>").count(), 3);
        assert!(text.contains("/d/"));
        assert!(text.contains("a.txt"));
        assert!(text.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_propfind_depth_infinity() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("d/sub")).unwrap();
        std::fs::write(dir.path().join("d/a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d/sub/b.txt"), b"b").unwrap();
        let app = make_app(&dir);

        let req = make_propfind("/d", "infinity", propfind_body("<D:resourcetype/>"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(text.matches("<D:response>").count(), 4);
    }

    #[tokio::test]
    async fn test_propfind_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let req = make_propfind("/ghost", "0", propfind_body("<D:resourcetype/>"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_propfind_allprop() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:allprop/></D:propfind>"#,
        );
        let req = make_propfind("/x.txt", "0", body);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        for p in &[
            "creationdate",
            "getcontentlength",
            "getcontenttype",
            "getetag",
            "getlastmodified",
            "lockdiscovery",
            "resourcetype",
            "supportedlock",
        ] {
            assert!(text.contains(p), "missing property: {p}");
        }
        assert!(!text.contains('\n'), "XML should have no newlines");
    }

    #[tokio::test]
    async fn test_propfind_unknown_prop() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), b"x").unwrap();
        let app = make_app(&dir);

        let req = make_propfind(
            "/x.txt",
            "0",
            propfind_body("<D:getcontentlength/><D:unknown-prop/>"),
        );
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("404 Not Found"));
        assert!(text.contains("unknown-prop"));
    }

    #[tokio::test]
    async fn test_propfind_href_encoding() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("my dir")).unwrap();
        std::fs::write(dir.path().join("my dir/file name.txt"), b"hi").unwrap();
        let app = make_app(&dir);

        let req = make_propfind("/my%20dir", "1", propfind_body("<D:resourcetype/>"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("file%20name.txt"));
    }

    #[tokio::test]
    async fn test_propfind_empty_body_defaults_to_allprop() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let req = make_propfind("/x.txt", "0", Body::empty());
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("getcontentlength"));
        assert!(text.contains("getlastmodified"));
        assert!(text.contains("resourcetype"));
    }

    // -- MKCOL tests --------------------------------------------------------

    fn make_app_mkcol(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle_mkcol))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
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
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/newdir");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        assert!(dir.path().join("newdir").is_dir());
    }

    #[tokio::test]
    async fn test_mkcol_parent_not_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/no_parent/newdir");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/d");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/f.txt");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_mkcol_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/../outside");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }

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
        // source still exists
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
