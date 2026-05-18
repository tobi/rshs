use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use axum::body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use crate::ok_or_return;
use crate::server::AppState;
use crate::utils::error::OrStatus;
use crate::utils::path;
use crate::webdav::{
    self,
    xml::{DAV_PREFIX, XmlWriterExt},
};

// ---------------------------------------------------------------------------
// PROPFIND
// ---------------------------------------------------------------------------

pub async fn handle_propfind(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let depth = webdav::parse_depth(req.headers());
    let request_path = req.uri().path().to_owned();

    let fs_path = match state.resolve_existing(&request_path).await {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed for PROPFIND");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let body_bytes = ok_or_return!(
        body::to_bytes(req.into_body(), 65536)
            .await
            .or_400("failed to read PROPFIND body")
    );
    let prop_request = ok_or_return!(
        webdav::parse_propfind_request(&body_bytes).or_400("failed to parse PROPFIND request")
    );

    let mut entries = webdav::fs::collect_entries(&fs_path, &request_path, depth).await;

    let dead_store = state.dead_props.read().await;
    let lock_store = state.locks.read().await;
    for entry in &mut entries {
        if let Some(ref cp) = entry.canonical_path {
            entry.dead_props = dead_store.get(cp).cloned();
            if let Some(locks) = lock_store.get(cp) {
                entry.active_locks = Some(locks.clone());
            }
        }
    }
    drop(dead_store);
    drop(lock_store);

    let xml = webdav::xml::build_multistatus(&entries, &prop_request);

    tracing::debug!(
        path = %fs_path.display(), depth = ?depth, entries = entries.len(), "PROPFIND completed"
    );

    webdav::xml::multistatus(xml)
}

// ---------------------------------------------------------------------------
// MKCOL
// ---------------------------------------------------------------------------

pub async fn handle_mkcol(State(state): State<Arc<AppState>>, req: Request) -> Response {
    // MKCOL MUST fail with 415 if the request has a body (RFC 2518 §8.3.1)
    let len = req.headers().get("content-length");
    if len.and_then(|v| v.to_str().ok()).is_some_and(|v| v != "0") {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }

    // MKCOL accepts trailing slashes per WebDAV client convention (e.g. litmus)
    let request_path = req.uri().path().trim_end_matches('/').to_owned();

    let target = match state.resolve_and_guard(&request_path).await {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!(error = %e, "path resolution failed for MKCOL");
            return e.status(StatusCode::FORBIDDEN).into_response();
        }
    };

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

// ---------------------------------------------------------------------------
// COPY / MOVE
// ---------------------------------------------------------------------------

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
    let depth = webdav::parse_depth(headers);

    let dest_str = match webdav::parse_destination(headers) {
        Some(s) => s,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };
    let src_path = req.uri().path().to_owned();

    let fs_src = match state.resolve_existing(&src_path).await {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let fs_dest = match state.resolve_write_target(&dest_str) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN.into_response(),
    };

    if fs_src == fs_dest {
        return StatusCode::FORBIDDEN.into_response();
    }

    let dest = match state.resolve_and_guard(&dest_str).await {
        Ok(t) => t,
        Err(path::ResolveTargetError::ParentCanonicalizeFailed(_)) => {
            tracing::debug!("dest parent not found for COPY/MOVE");
            return StatusCode::CONFLICT.into_response();
        }
        Err(path::ResolveTargetError::TraversalBlocked) => {
            return StatusCode::FORBIDDEN.into_response();
        }
        _ => unreachable!(),
    };
    let mut dest_existed = tokio::fs::metadata(&dest).await.is_ok();

    if dest_existed && !overwrite {
        tracing::debug!(verb, "target exists and Overwrite is F");
        return StatusCode::PRECONDITION_FAILED.into_response();
    }

    let meta = match tokio::fs::metadata(&fs_src).await {
        Ok(m) => m,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

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
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        } else if let Err(resp) = copy_dir(&fs_src, &dest, dest_existed).await {
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
    if dest_existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

async fn copy_file(src: &Path, dest: &Path) -> Result<(), Response> {
    tokio::fs::copy(src, dest)
        .await
        .or_500("copy file failed")?;
    Ok(())
}

async fn copy_dir(src: &Path, dest: &Path, dest_existed: bool) -> Result<(), Response> {
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
                if let Err(e) = tokio::fs::create_dir(&entry_dest).await {
                    if e.kind() != std::io::ErrorKind::AlreadyExists {
                        tracing::error!(error = %e, dest = %entry_dest.display(), "create sub dir failed");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                    }
                }
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

// ---------------------------------------------------------------------------
// PROPPATCH
// ---------------------------------------------------------------------------

pub async fn handle_proppatch(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = match state.resolve_existing(&request_path).await {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed for PROPPATCH");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let body_bytes = ok_or_return!(
        body::to_bytes(req.into_body(), 65536)
            .await
            .or_400("failed to read PROPPATCH body")
    );

    let op = ok_or_return!(
        webdav::parse_proppatch_request(&body_bytes).or_400("failed to parse PROPPATCH request")
    );

    let mut dead_props = state.dead_props.write().await;
    let entry = dead_props.entry(fs_path.clone()).or_default();

    let mut set_count = 0u32;
    let mut remove_count = 0u32;
    for action in &op.actions {
        match &action.value {
            Some(value) => {
                entry.insert(action.name.clone(), value.clone());
                set_count += 1;
            }
            None => {
                entry.remove(&action.name);
                remove_count += 1;
            }
        }
    }

    let xml = build_proppatch_response(&request_path, &op);

    tracing::debug!(path = %fs_path.display(), set = set_count, remove = remove_count, "PROPPATCH completed");

    drop(dead_props);

    webdav::xml::multistatus(xml)
}

fn build_proppatch_response(request_path: &str, op: &webdav::PropPatchOp) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
        .unwrap();

    let mut ms = BytesStart::new(format!("{DAV_PREFIX}multistatus"));
    ms.push_attribute(("xmlns:D", "DAV:"));
    writer.write_event(Event::Start(ms)).unwrap();

    for action in &op.actions {
        write_proppatch_result(&mut writer, request_path, &action.name, "200 OK");
    }

    writer
        .write_event(Event::End(BytesEnd::new(format!(
            "{DAV_PREFIX}multistatus"
        ))))
        .unwrap();

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

fn write_proppatch_result(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    href: &str,
    prop_name: &str,
    status: &str,
) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}response"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}href"))));
    writer.ev(Event::Text(BytesText::new(href)));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}href"))));

    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));
    let (ns, local) = webdav::parse_clark(prop_name).unwrap_or(("", prop_name));
    let mut elem = BytesStart::new(local);
    if !ns.is_empty() {
        elem.push_attribute(("xmlns", ns));
    }
    writer.ev(Event::Empty(elem));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));

    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::Text(BytesText::new(&format!("HTTP/1.1 {status}"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}response"))));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::{Router, body::Body, extract::Request, routing::any};
    use tower::ServiceExt;

    use crate::webdav;
    use crate::{AppState, AuthConfig};

    // -- PROPFIND tests -----------------------------------------------------

    fn make_app_propfind(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_propfind))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
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
        let app = make_app_propfind(&dir);

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
        let app = make_app_propfind(&dir);

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
        let app = make_app_propfind(&dir);

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
    async fn test_propfind_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_propfind(&dir);

        let req = make_propfind("/ghost", "0", propfind_body("<D:resourcetype/>"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
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
                AuthConfig::new(),
            )))
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
        Router::new()
            .fallback(any(super::handle_copy))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
    }

    fn make_copy_or_move(method: &[u8], uri: &str, dest: &str, overwrite: Option<&str>) -> Request {
        let mut builder = Request::builder()
            .method(axum::http::Method::from_bytes(method).unwrap())
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
        Router::new()
            .fallback(any(super::handle_move))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
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

    // -- PROPPATCH tests ----------------------------------------------------

    fn make_app_proppatch(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_proppatch))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
    }

    #[tokio::test]
    async fn test_proppatch_set_and_read_back() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let app = make_app_proppatch(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:author>Alice</X:author></D:prop></D:set></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("200 OK"));
        assert!(text.contains("author"));
    }

    #[tokio::test]
    async fn test_proppatch_remove() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let app = make_app_proppatch(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:tag>important</X:tag></D:prop></D:set></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let app = make_app_proppatch(&dir);
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:remove><D:prop><X:tag/></D:prop></D:remove></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);
    }

    #[tokio::test]
    async fn test_proppatch_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_proppatch(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:foo>bar</X:foo></D:prop></D:set></D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/ghost.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_proppatch_bad_xml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        let app = make_app_proppatch(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(Body::from("not xml"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    fn make_app_combined(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(
                |State(state): State<Arc<AppState>>, req: Request| async move {
                    if req.method() == &*webdav::M_PROPFIND {
                        super::handle_propfind(State(state), req).await
                    } else if req.method() == &*webdav::M_PROPPATCH {
                        super::handle_proppatch(State(state), req).await
                    } else {
                        StatusCode::METHOD_NOT_ALLOWED.into_response()
                    }
                },
            ))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
    }

    #[tokio::test]
    async fn test_propfind_invalid_xml_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_propfind(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/")
            .header("depth", "0")
            .body(Body::from("<foo>"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
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
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><prop0 xmlns="http://example.com/neon/litmus/"/></D:prop></D:propfind>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPFIND").unwrap())
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
