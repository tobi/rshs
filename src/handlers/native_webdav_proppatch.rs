use std::sync::Arc;

use axum::{
    body::{self, Body},
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use crate::server::AppState;
use crate::utils::path;
use crate::webdav;
use crate::webdav::xml::DAV_PREFIX;

pub async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &request_path).await {
            Some(p) => p,
            None => {
                tracing::debug!("path resolution failed for PROPPATCH");
                return StatusCode::NOT_FOUND.into_response();
            }
        };

    let body_bytes = match body::to_bytes(req.into_body(), 65536).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "failed to read PROPPATCH body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let op = match webdav::parse_proppatch_request(&body_bytes) {
        Ok(o) => o,
        Err(e) => {
            tracing::debug!(error = %e, "failed to parse PROPPATCH request");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let mut dead_props = state.dead_props.write().await;
    let entry = dead_props.entry(fs_path.clone()).or_default();

    // Removed props → 200 OK
    for name in &op.remove {
        entry.remove(name);
    }

    // Set props
    for (name, value) in &op.set {
        entry.insert(name.clone(), value.clone());
    }

    let xml = build_proppatch_response(&request_path, &op);

    tracing::debug!(path = %fs_path.display(), set = op.set.len(), remove = op.remove.len(), "PROPPATCH completed");

    drop(dead_props);

    Response::builder()
        .status(StatusCode::from_u16(207).unwrap())
        .header("content-type", "application/xml; charset=utf-8")
        .body(Body::from(xml))
        .unwrap()
}

fn build_proppatch_response(request_path: &str, op: &webdav::PropPatchOp) -> String {
    let mut writer = Writer::new(std::io::Cursor::new(Vec::new()));

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
        .unwrap();

    let mut ms = BytesStart::new(format!("{DAV_PREFIX}multistatus"));
    ms.push_attribute(("xmlns:D", "DAV:"));
    writer.write_event(Event::Start(ms)).unwrap();

    // One response per set prop
    for name in op.set.keys() {
        write_proppatch_result(&mut writer, request_path, name, "200 OK");
    }
    for name in &op.remove {
        write_proppatch_result(&mut writer, request_path, name, "200 OK");
    }

    writer
        .write_event(Event::End(BytesEnd::new(format!(
            "{DAV_PREFIX}multistatus"
        ))))
        .unwrap();

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

fn write_proppatch_result(
    writer: &mut Writer<std::io::Cursor<Vec<u8>>>,
    href: &str,
    prop_name: &str,
    status: &str,
) {
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}response"
        ))))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}href"))))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new(href)))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}href"))))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}propstat"
        ))))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))))
        .unwrap();
    writer
        .write_event(Event::Empty(BytesStart::new(prop_name)))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new(&format!("HTTP/1.1 {status}"))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))))
        .unwrap();

    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}response"))))
        .unwrap();
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
                dead_props: Arc::new(tokio::sync::RwLock::new(
                    crate::webdav::DeadPropertyStore::new(),
                )),
                locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
            }))
    }

    #[tokio::test]
    async fn test_proppatch_set_and_read_back() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"hello").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:set><D:prop><X:author>Alice</X:author></D:prop></D:set>
</D:propertyupdate>"#,
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
        let app = make_app(&dir);

        // First SET
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:set><D:prop><X:tag>important</X:tag></D:prop></D:set>
</D:propertyupdate>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 207);

        // Re-create app since oneshot consumes it
        let app = make_app(&dir);

        // Then REMOVE
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:remove><D:prop><X:tag/></D:prop></D:remove>
</D:propertyupdate>"#,
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
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:set><D:prop><X:foo>bar</X:foo></D:prop></D:set>
</D:propertyupdate>"#,
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
        let app = make_app(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"PROPPATCH").unwrap())
            .uri("/f.txt")
            .body(Body::from("not xml"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
