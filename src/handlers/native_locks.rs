use std::io::Cursor;
use std::sync::Arc;

use axum::{
    body::{self, Body},
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::server::AppState;
use crate::utils::path;
use crate::webdav;
use crate::webdav::xml::DAV_PREFIX;

pub async fn handle_lock(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &request_path).await {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND.into_response(),
        };

    let timeout = webdav::parse_timeout(req.headers());
    let body_bytes = match body::to_bytes(req.into_body(), 65536).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let owner = parse_lock_owner(&body_bytes);
    let token = webdav::generate_lock_token();

    let lock = webdav::LockInfo {
        token: token.clone(),
        scope: webdav::LockScope::Exclusive,
        owner,
        timeout,
        created: std::time::SystemTime::now(),
        depth: webdav::Depth::Zero,
    };

    let mut locks = state.locks.write().await;
    let entry = locks.entry(fs_path.clone()).or_default();

    // Check if already locked (refresh)
    let existed = entry
        .iter()
        .any(|l| matches!(l.scope, webdav::LockScope::Exclusive));
    if existed {
        entry.retain(|l| !matches!(l.scope, webdav::LockScope::Exclusive));
    }
    entry.push(lock);

    let xml = build_lock_response(&token, timeout);

    tracing::debug!(path = %fs_path.display(), token = %token, "LOCK completed");

    drop(locks);

    let status = StatusCode::OK;

    Response::builder()
        .status(status)
        .header("content-type", "application/xml; charset=utf-8")
        .header("lock-token", format!("<{token}>"))
        .body(Body::from(xml))
        .unwrap()
}

pub async fn handle_unlock(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();
    let token = match webdav::parse_lock_token_header(req.headers()) {
        Some(t) => t,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };

    let fs_path =
        match path::resolve_existing(&state.root_dir, &state.root_canonical, &request_path).await {
            Some(p) => p,
            None => return StatusCode::NOT_FOUND.into_response(),
        };

    let mut locks = state.locks.write().await;
    if let Some(entry) = locks.get_mut(&fs_path) {
        let before = entry.len();
        entry.retain(|l| l.token != token);
        if entry.len() < before {
            tracing::debug!(path = %fs_path.display(), token = %token, "UNLOCK completed");
            drop(locks);
            return StatusCode::NO_CONTENT.into_response();
        }
    }
    drop(locks);
    StatusCode::FORBIDDEN.into_response()
}

fn parse_lock_owner(xml: &[u8]) -> Option<String> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut in_owner = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"owner" => {
                in_owner = true;
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"owner" => {
                in_owner = false;
            }
            Ok(Event::Text(t)) if in_owner => {
                return Some(String::from_utf8_lossy(t.as_ref()).to_string());
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

fn build_lock_response(token: &str, timeout: Option<std::time::Duration>) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}lockdiscovery"
        ))))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}activelock"
        ))))
        .unwrap();

    // lockscope
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}lockscope"
        ))))
        .unwrap();
    writer
        .write_event(Event::Empty(BytesStart::new(format!(
            "{DAV_PREFIX}exclusive"
        ))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockscope"))))
        .unwrap();

    // locktype
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}locktype"
        ))))
        .unwrap();
    writer
        .write_event(Event::Empty(BytesStart::new(format!("{DAV_PREFIX}write"))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktype"))))
        .unwrap();

    // depth
    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}depth"))))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new("0")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}depth"))))
        .unwrap();

    // timeout
    if let Some(d) = timeout {
        writer
            .write_event(Event::Start(BytesStart::new(format!(
                "{DAV_PREFIX}timeout"
            ))))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(&format!(
                "Second-{}",
                d.as_secs()
            ))))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}timeout"))))
            .unwrap();
    }

    // locktoken
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}locktoken"
        ))))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}href"))))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new(token)))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}href"))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktoken"))))
        .unwrap();

    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}activelock"))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!(
            "{DAV_PREFIX}lockdiscovery"
        ))))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))))
        .unwrap();

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
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
            .fallback(any(super::handle_lock))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
                locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
            }))
    }

    fn make_app_unlock(dir: &tempfile::TempDir) -> Router {
        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .fallback(any(super::handle_unlock))
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
    async fn test_lock_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
</D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(
            resp.headers()
                .get("lock-token")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("opaquelocktoken")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("activelock"));
    }

    #[tokio::test]
    async fn test_lock_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
</D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/ghost.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_lock_refresh() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
</D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Re-create app and send LOCK again (refresh)
        let app = make_app(&dir);
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
</D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_unlock() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();

        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let state = Arc::new(AppState {
            root_dir: root.clone(),
            root_canonical: canonical,
            dav_handler: crate::handlers::dav_fallback::create_dav_handler(&root),
            auth_config: Arc::new(AuthConfig::new()),
            dead_props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            locks: Arc::new(tokio::sync::RwLock::new(crate::webdav::LockStore::new())),
        });
        let lock_app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());
        let unlock_app = Router::new()
            .fallback(any(super::handle_unlock))
            .with_state(state);

        // Lock
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
</D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = lock_app.oneshot(req).await.unwrap();
        let token = resp
            .headers()
            .get("lock-token")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('<')
            .trim_matches('>')
            .to_string();

        // Unlock
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"UNLOCK").unwrap())
            .uri("/f.txt")
            .header("lock-token", format!("<{token}>"))
            .body(Body::empty())
            .unwrap();
        let resp = unlock_app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_unlock_wrong_token() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app_unlock(&dir);

        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"UNLOCK").unwrap())
            .uri("/f.txt")
            .header("lock-token", "<opaquelocktoken:bad>")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }
}
