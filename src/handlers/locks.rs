use std::io::Cursor;
use std::sync::Arc;

use axum::body::{self, Body};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use crate::ok_or_return;
use crate::server::AppState;
use crate::utils::error::{IntoResolved, OrStatus};
use crate::webdav::{
    self,
    xml::{XmlWriterExt, dav_qname, write_activelock},
};

// ---------------------------------------------------------------------------
// LOCK
// ---------------------------------------------------------------------------

pub async fn handle_lock(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().trim_end_matches('/').to_owned();

    let target = state.resolve_and_guard(&request_path).await;
    let target = ok_or_return!(target.or_invalid(StatusCode::FORBIDDEN));

    let timeout = webdav::parse_timeout(req.headers());
    let depth = webdav::parse_depth(req.headers());
    let if_entries = webdav::parse_if_header(req.headers());
    let if_tokens: Vec<String> = if_entries
        .iter()
        .flat_map(|e| e.positive_tokens_iter())
        .map(|t| t.to_string())
        .collect();
    let body_bytes = body::to_bytes(req.into_body(), 65536).await;
    let body_bytes = ok_or_return!(body_bytes.or_400("failed to read LOCK body"));

    let (owner, lock_scope) = parse_lock_body(&body_bytes);

    let mut locks = state.locks.write().await;

    if let Some(al) = webdav::find_ancestor_lock(&locks, &target, &state.root_canonical, |l| {
        if_tokens.contains(&l.token)
    }) {
        let al = al.clone();
        let xml = build_lock_response(&al);
        tracing::debug!(
            path = %target.display(),
            token = %al.token,
            ancestor = true,
            "indirect LOCK refresh via ancestor depth:infinity lock"
        );
        drop(locks);
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/xml; charset=utf-8")
            .header("lock-token", format!("<{}>", al.token))
            .body(Body::from(xml))
            .unwrap();
    }

    let entry = locks.entry(target.clone()).or_default();

    let (token, is_refresh) = match lock_scope {
        webdav::LockScope::Exclusive => {
            match try_acquire_exclusive(entry, &if_tokens, &target).await {
                Ok(v) => v,
                Err(status) => {
                    drop(locks);
                    return status.into_response();
                }
            }
        }
        webdav::LockScope::Shared => match try_acquire_shared(entry, &if_tokens, &target).await {
            Ok(v) => v,
            Err(status) => {
                drop(locks);
                return status.into_response();
            }
        },
    };

    let lock = webdav::LockInfo {
        token: token.clone(),
        scope: lock_scope,
        owner,
        timeout,
        created: std::time::SystemTime::now(),
        depth,
    };
    let xml = build_lock_response(&lock);
    entry.push(lock);

    tracing::debug!(path = %target.display(), token = %token, is_refresh, "LOCK completed");

    drop(locks);

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/xml; charset=utf-8")
        .header("lock-token", format!("<{token}>"))
        .body(Body::from(xml))
        .unwrap()
}

// ---------------------------------------------------------------------------
// UNLOCK
// ---------------------------------------------------------------------------

pub async fn handle_unlock(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let token = webdav::parse_lock_token_header(req.headers());
    let token = ok_or_return!(token.or_400("missing or invalid lock-token header for UNLOCK"));

    let fs_path = state.resolve_existing(&request_path).await;
    let fs_path = ok_or_return!(fs_path.or_404("resource not found for UNLOCK"));

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

// ---------------------------------------------------------------------------
// Lock handling logic
// ---------------------------------------------------------------------------

fn parse_lock_body(xml: &[u8]) -> (Option<String>, webdav::LockScope) {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut in_owner = false;
    let mut in_lockscope = false;
    let mut owner = None;
    let mut scope = webdav::LockScope::Exclusive;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    b"owner" => in_owner = true,
                    b"lockscope" => in_lockscope = true,
                    b"shared" if in_lockscope => scope = webdav::LockScope::Shared,
                    b"exclusive" if in_lockscope => {}
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    b"owner" => in_owner = false,
                    b"lockscope" => in_lockscope = false,
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_owner => {
                owner = Some(String::from_utf8_lossy(t.as_ref()).to_string());
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (owner, scope)
}

fn build_lock_response(lock: &webdav::LockInfo) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    let mut prop = BytesStart::new(dav_qname("prop"));
    prop.push_attribute(("xmlns:D", "DAV:"));
    writer.ev(Event::Start(prop));

    writer.ev(Event::Start(BytesStart::new(dav_qname("lockdiscovery"))));

    write_activelock(&mut writer, lock);

    writer.ev(Event::End(BytesEnd::new(dav_qname("lockdiscovery"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("prop"))));

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

async fn ensure_lock_null_resource(target: &std::path::Path) -> Result<(), StatusCode> {
    if tokio::fs::metadata(target).await.is_ok() {
        tracing::debug!(path = %target.display(), "lock-null resource already exists");
        return Ok(());
    }

    match tokio::fs::File::create(target).await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "failed to create lock-null resource"
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn check_existing_exclusive(
    entry: &[webdav::LockInfo],
    if_tokens: &[String],
) -> Result<Option<String>, StatusCode> {
    let token = entry
        .iter()
        .find(|l| l.is_exclusive())
        .map(|l| l.token.clone());
    match token {
        Some(t) if if_tokens.contains(&t) => Ok(Some(t)),
        Some(_) => Err(StatusCode::LOCKED),
        None => Ok(None),
    }
}

async fn try_acquire_exclusive(
    entry: &mut Vec<webdav::LockInfo>,
    if_tokens: &[String],
    target: &std::path::Path,
) -> Result<(String, bool), StatusCode> {
    if let Some(token) = check_existing_exclusive(entry, if_tokens)? {
        entry.retain(|l| !l.is_exclusive());
        return Ok((token, true));
    }

    if !entry.is_empty() {
        if entry.iter().all(|l| if_tokens.contains(&l.token)) {
            entry.clear();
            return Ok((webdav::generate_lock_token(), false));
        }
        return Err(StatusCode::LOCKED);
    }

    ensure_lock_null_resource(target).await?;
    Ok((webdav::generate_lock_token(), false))
}

async fn try_acquire_shared(
    entry: &mut Vec<webdav::LockInfo>,
    if_tokens: &[String],
    target: &std::path::Path,
) -> Result<(String, bool), StatusCode> {
    if check_existing_exclusive(entry, if_tokens)?.is_some() {
        entry.retain(|l| !l.is_exclusive());
        return Ok((webdav::generate_lock_token(), false));
    }

    if let Some(existing) = entry.iter().find(|l| if_tokens.contains(&l.token)) {
        let token = existing.token.clone();
        entry.retain(|l| l.token != token);
        return Ok((token, true));
    }

    ensure_lock_null_resource(target).await?;
    Ok((webdav::generate_lock_token(), false))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request, routing::any};
    use tower::ServiceExt;

    use crate::{AppState, AuthConfig};

    fn make_app(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_lock))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
    }

    fn make_app_unlock(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(any(super::handle_unlock))
            .with_state(std::sync::Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
            )))
    }

    #[tokio::test]
    async fn test_lock_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
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
    async fn test_lock_creates_locknull() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/ghost.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // RFC 4918 §7.3: LOCK on non-existent URL creates lock-null resource
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(dir.path().join("ghost.txt").exists());
    }

    #[tokio::test]
    async fn test_lock_refresh() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
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
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
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

        let state = Arc::new(AppState::new(dir.path().to_path_buf(), AuthConfig::new()));
        let lock_app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());
        let unlock_app = Router::new()
            .fallback(any(super::handle_unlock))
            .with_state(state);

        // Lock
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
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

    #[tokio::test]
    async fn test_lock_shared() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let app = make_app(&dir);

        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:shared/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(text.contains("<D:shared"));
    }

    #[tokio::test]
    async fn test_shared_lock_blocks_exclusive() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf(), AuthConfig::new()));
        let app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());
        let app2 = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state);

        // Shared lock
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:shared/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Exclusive lock from different client → 423
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::LOCKED);
    }

    #[tokio::test]
    async fn test_exclusive_lock_blocks_shared() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf(), AuthConfig::new()));
        let app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());
        let app2 = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state);

        // Exclusive lock
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Shared lock from different client → 423
        let body = Body::from(
            r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:shared/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#,
        );
        let req = Request::builder()
            .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
            .uri("/f.txt")
            .body(body)
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::LOCKED);
    }

    #[tokio::test]
    async fn test_double_shared_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf(), AuthConfig::new()));
        let app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());
        let app2 = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state);

        let shared_body = r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:shared/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#;

        // First shared lock
        let resp = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
                    .uri("/f.txt")
                    .body(Body::from(shared_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Second shared lock from different client → should succeed
        let resp = app2
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
                    .uri("/f.txt")
                    .body(Body::from(shared_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_shared_lock_refresh() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf(), AuthConfig::new()));
        let app = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state.clone());

        let shared_body = r#"<?xml version="1.0" encoding="utf-8"?><D:lockinfo xmlns:D="DAV:"><D:lockscope><D:shared/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockinfo>"#;

        // First shared lock
        let resp = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
                    .uri("/f.txt")
                    .body(Body::from(shared_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let token = resp
            .headers()
            .get("lock-token")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('<')
            .trim_matches('>')
            .to_string();

        // Refresh with token
        let app2 = Router::new()
            .fallback(any(super::handle_lock))
            .with_state(state);
        let resp = app2
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::from_bytes(b"LOCK").unwrap())
                    .uri("/f.txt")
                    .header("if", format!("(<{token}>)"))
                    .body(Body::from(shared_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        // Same token returned on refresh
        let refreshed = resp
            .headers()
            .get("lock-token")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('<')
            .trim_matches('>');
        assert_eq!(refreshed, token);
    }
}
