//! PROPFIND, MKCOL, COPY, MOVE, and PROPPATCH WebDAV protocol handlers.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use axum::body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use crate::server::{AppResult, AppState};
use crate::utils::error::{IntoResolved, OrStatus};
use crate::webdav::{self, El, XmlWriter, XmlWriterExt};

/// PROPFIND handler — returns resource properties (RFC 4918 §9.1).
///
/// Supports `Depth: 0`, `1`, and `infinity`. Accepts `allprop`, `propname`,
/// and named property requests. Returns a `207 Multi-Status` XML response.
pub async fn handle_propfind(State(state): State<Arc<AppState>>, req: Request) -> AppResult {
    let depth = webdav::parse_depth(req.headers());
    let request_path = req.uri().path().to_owned();

    let fs_path = state.resolve_existing(&request_path).await;
    let fs_path = fs_path.or_404("path resolution failed for PROPFIND")?;

    let body_bytes = body::to_bytes(req.into_body(), 65536).await;
    let body_bytes = body_bytes.or_400("failed to read PROPFIND body")?;

    let prop_request =
        webdav::parse_propfind_request(&body_bytes).or_400("failed to parse PROPFIND request")?;

    let mut entries = webdav::fs::collect_entries(&fs_path, &request_path, depth).await;

    let dead_store = state.dead_props.read().await;
    let lock_store = state.locks.read().await;
    for entry in &mut entries {
        if let Some(ref cp) = entry.canonical_path {
            entry.dead_props = dead_store.get(cp).cloned();
            if let Some(locks) = lock_store.get(cp) {
                let active: Vec<_> = webdav::ls::active_slice(locks).cloned().collect();
                if !active.is_empty() {
                    entry.active_locks = Some(active);
                }
            }
        }
    }
    drop(dead_store);
    drop(lock_store);

    let xml = webdav::xml::build_multistatus(&entries, &prop_request);

    tracing::debug!(
        path = %fs_path.display(), depth = ?depth, entries = entries.len(), "PROPFIND completed"
    );

    Ok(webdav::xml::multistatus(xml))
}

/// MKCOL handler — creates a new collection (directory) (RFC 4918 §9.3).
///
/// Returns `201 Created` on success. Rejects if the parent does not exist,
/// a file already occupies the path, or the target is root.
pub async fn handle_mkcol(State(state): State<Arc<AppState>>, req: Request) -> AppResult {
    // MKCOL MUST fail with 415 if the request has a body (RFC 2518 §8.3.1)
    let len = req.headers().get("content-length");
    if len.and_then(|v| v.to_str().ok()).is_some_and(|v| v != "0") {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // MKCOL accepts trailing slashes per WebDAV client convention (e.g. litmus)
    let request_path = req.uri().path().trim_end_matches('/').to_owned();

    let target = state.resolve_and_guard(&request_path).await;
    let target = target.or_invalid(StatusCode::FORBIDDEN)?;

    if tokio::fs::metadata(&target).await.is_ok() {
        tracing::debug!(path = %target.display(), "MKCOL target already exists");
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    match tokio::fs::create_dir(&target).await {
        Ok(()) => {
            tracing::debug!(path = %target.display(), "MKCOL completed");
            Ok(StatusCode::CREATED.into_response())
        }
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "failed to create directory for MKCOL"
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// COPY handler — duplicates a resource to a destination (RFC 4918 §9.8).
///
/// Supports recursive directory copy. Respects the `Overwrite` header.
/// Returns `201 Created` for new destinations, `204 No Content` for overwrites.
pub async fn handle_copy(State(state): State<Arc<AppState>>, req: Request) -> AppResult {
    do_move_or_copy(&state, req, false).await
}

/// MOVE handler — relocates a resource to a destination (RFC 4918 §9.9).
///
/// Supports recursive directory moves. Respects the `Overwrite` header.
/// Equivalent to COPY + DELETE when source and destination share a filesystem.
pub async fn handle_move(State(state): State<Arc<AppState>>, req: Request) -> AppResult {
    do_move_or_copy(&state, req, true).await
}

async fn do_move_or_copy(state: &Arc<AppState>, req: Request, is_move: bool) -> AppResult {
    let verb = if is_move { "MOVE" } else { "COPY" };
    let headers = req.headers();
    let overwrite = webdav::parse_overwrite(headers);
    let depth = webdav::parse_depth(headers);

    let dest_str = webdav::parse_destination(headers);
    let dest_str = dest_str.or_400("missing or invalid Destination header")?;

    let src_path = req.uri().path().to_owned();

    let fs_src = state.resolve_existing(&src_path).await;
    let fs_src = fs_src.or_404("source not found")?;

    let fs_dest = state.resolve_write_target(&dest_str);
    let fs_dest = fs_dest.or_403("invalid destination path")?;

    if fs_src == fs_dest {
        tracing::debug!(verb, "source and destination are the same");
        return Err(StatusCode::FORBIDDEN);
    }

    let dest = state.resolve_and_guard(&dest_str).await;
    let dest = dest.or_invalid(StatusCode::BAD_REQUEST)?;
    let dest_existed_before = tokio::fs::metadata(&dest).await.is_ok();
    let mut dest_existed = dest_existed_before;

    if dest_existed && !overwrite {
        tracing::debug!(verb, "target exists and Overwrite is F");
        return Err(StatusCode::PRECONDITION_FAILED);
    }

    let meta = tokio::fs::metadata(&fs_src).await;
    let meta = meta.or_404("source not found for COPY/MOVE")?;

    // Type-incompatible overwrite with Overwrite:T — clean up target first
    if overwrite && dest_existed {
        if let Ok(dest_meta) = tokio::fs::metadata(&dest).await {
            if !meta.is_dir() && dest_meta.is_dir() {
                let _ = tokio::fs::remove_dir_all(&dest).await;
                dest_existed = false;
            } else if meta.is_dir() && dest_meta.is_file() {
                let _ = tokio::fs::remove_file(&dest).await;
                dest_existed = false;
            }
        }
    }

    if meta.is_dir() {
        // MOVE ignores Depth header; COPY with Depth:0 makes shallow copy
        if !is_move && depth == webdav::Depth::Zero {
            if !dest_existed {
                if let Err(e) = tokio::fs::create_dir(&dest).await {
                    tracing::error!(error = %e, "shallow copy create dir failed");
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        } else {
            copy_dir(&fs_src, &dest, dest_existed).await?;
        }
    } else {
        copy_file(&fs_src, &dest).await?;
    }

    if is_move && tokio::fs::rename(&fs_src, &dest).await.is_err() {
        if meta.is_dir() {
            let _ = tokio::fs::remove_dir_all(&fs_src).await;
        } else {
            let _ = tokio::fs::remove_file(&fs_src).await;
        }
    }

    // Migrate dead properties for COPY/MOVE
    let mut dead_props = state.dead_props.write().await;
    if let Some(props) = dead_props.remove(&fs_src) {
        if !is_move {
            dead_props.insert(fs_src.clone(), props.clone());
        }
        dead_props.insert(dest.clone(), props);
    }
    drop(dead_props);

    tracing::debug!(verb, src = %fs_src.display(), dest = %dest.display(), "completed");

    if dest_existed_before {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(StatusCode::CREATED.into_response())
    }
}

async fn copy_file(src: &Path, dest: &Path) -> Result<(), StatusCode> {
    tokio::fs::copy(src, dest)
        .await
        .or_500("copy file failed")?;
    Ok(())
}

async fn copy_dir(src: &Path, dest: &Path, dest_existed: bool) -> Result<(), StatusCode> {
    if !dest_existed {
        tokio::fs::create_dir(dest).await.map_err(|e| {
            tracing::error!(error = %e, dest = %dest.display(), "create dest dir failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    let mut stack = vec![(src.to_path_buf(), dest.to_path_buf())];
    while let Some((src_dir, dest_dir)) = stack.pop() {
        let mut read_dir = tokio::fs::read_dir(&src_dir).await.map_err(|e| {
            tracing::error!(error = %e, dir = %src_dir.display(), "read dir failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            tracing::error!(error = %e, "read entry failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })? {
            let file_type = entry.file_type().await.map_err(|e| {
                tracing::error!(error = %e, "file_type failed");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            let entry_dest = dest_dir.join(entry.file_name());

            if file_type.is_dir() {
                if let Err(e) = tokio::fs::create_dir(&entry_dest).await {
                    if e.kind() != std::io::ErrorKind::AlreadyExists {
                        tracing::error!(error = %e, dest = %entry_dest.display(), "create sub dir failed");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                }
                stack.push((entry.path(), entry_dest));
            } else if file_type.is_symlink() {
                continue;
            } else {
                tokio::fs::copy(entry.path(), &entry_dest).await.map_err(|e| {
                    tracing::error!(
                        error = %e, src = %entry.path().display(), dest = %entry_dest.display(), "copy file failed"
                    );
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            }
        }
    }
    Ok(())
}

/// PROPPATCH handler — sets and/or removes dead properties (RFC 4918 §9.2).
///
/// Processes `set` and `remove` actions from the request body. Returns a
/// `207 Multi-Status` response with per-property success/failure status.
pub async fn handle_proppatch(State(state): State<Arc<AppState>>, req: Request) -> AppResult {
    let request_path = req.uri().path().to_owned();

    let fs_path = state.resolve_existing(&request_path).await;
    let fs_path = fs_path.or_404("path resolution failed for PROPPATCH")?;

    let body_bytes = body::to_bytes(req.into_body(), 65536).await;
    let body_bytes = body_bytes.or_400("failed to read PROPPATCH body")?;

    let op =
        webdav::parse_proppatch_request(&body_bytes).or_400("failed to parse PROPPATCH request")?;

    let mut dead_props = state.dead_props.write().await;
    let entry = dead_props.entry(fs_path.clone()).or_default();

    let mut set_count = 0u32;
    let mut remove_count = 0u32;
    for action in &op.actions {
        match &action.1 {
            Some(value) => {
                entry.insert(action.0.clone(), value.clone());
                set_count += 1;
            }
            None => {
                entry.remove(&action.0);
                remove_count += 1;
            }
        }
    }

    let xml = build_proppatch_response(&request_path, &op);

    tracing::debug!(
        path = %fs_path.display(), set = set_count, remove = remove_count, "PROPPATCH completed"
    );

    drop(dead_props);

    Ok(webdav::xml::multistatus(xml))
}

fn build_proppatch_response(request_path: &str, op: &webdav::PropPatchOp) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer.ev(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut ms = BytesStart::new(El::MULTISTATUS);

    ms.push_attribute(("xmlns:D", "DAV:"));

    writer.ev(Event::Start(ms));

    for action in &op.actions {
        write_proppatch_result(&mut writer, request_path, &action.0, "200 OK");
    }

    writer.ev(Event::End(BytesEnd::new(El::MULTISTATUS)));

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

fn write_proppatch_result(writer: &mut XmlWriter, href: &str, prop_name: &str, status: &str) {
    writer.ev(Event::Start(BytesStart::new(El::RESPONSE)));

    writer.ev(Event::Start(BytesStart::new(El::HREF)));
    writer.ev(Event::Text(BytesText::new(href)));
    writer.ev(Event::End(BytesEnd::new(El::HREF)));

    writer.ev(Event::Start(BytesStart::new(El::PROPSTAT)));

    writer.ev(Event::Start(BytesStart::new(El::PROP)));
    let (ns, local) = webdav::parse_clark(prop_name).unwrap_or(("", prop_name));
    let mut elem = BytesStart::new(local);
    if !ns.is_empty() {
        elem.push_attribute(("xmlns", ns));
    }
    writer.ev(Event::Empty(elem));
    writer.ev(Event::End(BytesEnd::new(El::PROP)));

    writer.ev(Event::Start(BytesStart::new(El::STATUS)));
    writer.ev(Event::Text(BytesText::new(&format!("HTTP/1.1 {status}"))));
    writer.ev(Event::End(BytesEnd::new(El::STATUS)));

    writer.ev(Event::End(BytesEnd::new(El::PROPSTAT)));
    writer.ev(Event::End(BytesEnd::new(El::RESPONSE)));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::{Method as HttpMethod, StatusCode};
    use axum::{Router, body::Body, extract::Request, routing::any};
    use tower::ServiceExt;

    use crate::webdav::Method;
    use crate::{AppState, AuthState};

    // -- PROPFIND tests -----------------------------------------------------

    fn make_app_propfind(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_propfind))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    fn propfind_body(props: &str) -> Body {
        Body::from(format!(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop>{}</D:prop></D:propfind>"#,
            props
        ))
    }

    fn make_propfind(uri: &str, depth: &str, body: Body) -> Request {
        Request::builder()
            .method(HttpMethod::from_bytes(b"PROPFIND").unwrap())
            .uri(uri)
            .header("depth", depth)
            .body(body)
            .unwrap()
    }

    #[tokio::test]
    async fn test_propfind_depth_infinity() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("d/sub")).unwrap();
        std::fs::write(dir.path().join("d/a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d/sub/b.txt"), b"b").unwrap();
        let app = make_app_propfind(&dir);

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
    async fn test_propfind_allprop() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("x.txt"), b"data").unwrap();
        let app = make_app_propfind(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:allprop/></D:propfind>"#,
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
        let app = make_app_propfind(&dir);

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
        let app = make_app_propfind(&dir);

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
        let app = make_app_propfind(&dir);

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
        Router::new()
            .fallback(any(super::handle_mkcol))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    fn make_mkcol(uri: &str) -> Request {
        Request::builder()
            .method(HttpMethod::from_bytes(b"MKCOL").unwrap())
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_mkcol_parent_not_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/no_parent/newdir");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("d")).unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/d");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_already_exists_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/f.txt");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcol_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_mkcol_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_mkcol(&dir);

        let req = make_mkcol("/../outside");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -- COPY tests ---------------------------------------------------------

    fn make_app_copy(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_copy))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    fn make_copy_or_move(method: &[u8], uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .method(HttpMethod::from_bytes(method).unwrap())
            .uri(uri)
            .header("destination", dest);
        if let Some(ov) = overwrite {
            builder = builder.header("overwrite", ov);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn make_copy(uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        make_copy_or_move(b"COPY", uri, dest, overwrite)
    }

    fn make_move(uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        make_copy_or_move(b"MOVE", uri, dest, overwrite)
    }

    #[tokio::test]
    async fn test_copy_overwrite_false() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"b").unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/s.txt", "http://x/d.txt", Some("F"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_copy_source_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_copy(&dir);

        let req = make_copy("/ghost", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_copy_no_dest_header() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"x").unwrap();
        let app = make_app_copy(&dir);

        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"COPY").unwrap())
            .uri("/s.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // -- MOVE tests ---------------------------------------------------------

    fn make_app_move(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_move))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    #[tokio::test]
    async fn test_move_overwrite_false() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("d.txt"), b"b").unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/s.txt", "http://x/d.txt", Some("F"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_move_source_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_move(&dir);

        let req = make_move("/ghost", "http://x/d.txt", None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_move_no_dest_header() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("s.txt"), b"x").unwrap();
        let app = make_app_move(&dir);

        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"MOVE").unwrap())
            .uri("/s.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // -- PROPPATCH tests ----------------------------------------------------

    fn make_app_proppatch(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_proppatch))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    #[tokio::test]
    async fn test_proppatch_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_proppatch(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:foo>bar</X:foo></D:prop></D:set></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"PROPPATCH").unwrap())
            .uri("/ghost.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_proppatch_bad_xml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        let app = make_app_proppatch(&dir);

        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(Body::from("not xml"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    fn make_app_combined(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(
                |State(state): State<Arc<AppState>>, req: Request| async move {
                    let method = Method::try_from(req.method()).unwrap();
                    if method == Method::PROPFIND {
                        super::handle_propfind(State(state), req).await
                    } else if method == Method::PROPPATCH {
                        super::handle_proppatch(State(state), req).await
                    } else {
                        Err(StatusCode::METHOD_NOT_ALLOWED)
                    }
                },
            ))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthState::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    #[tokio::test]
    async fn test_propfind_invalid_xml_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_propfind(&dir);

        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"PROPFIND").unwrap())
            .uri("/")
            .header("depth", "0")
            .body(Body::from("<foo>"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_proppatch_namespace_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let app = make_app_combined(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><prop0 xmlns="http://example.com/neon/litmus/">value0</prop0></D:prop></D:set></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><prop0 xmlns="http://example.com/neon/litmus/"/></D:prop></D:propfind>"#,
        );
        let req = Request::builder()
            .method(HttpMethod::from_bytes(b"PROPFIND").unwrap())
            .uri("/f.txt")
            .header("depth", "0")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("xmlns=\"http://example.com/neon/litmus/\""));
        assert!(text.contains(">value0<"));
    }
}
