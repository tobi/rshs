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
        path = %request_path, depth = ?depth, entries = entries.len(), "PROPFIND completed"
    );

    Response::builder()
        .status(StatusCode::from_u16(207).unwrap())
        .header("content-type", "application/xml; charset=utf-8")
        .body(Body::from(xml))
        .unwrap()
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
        assert!(text.contains("getcontentlength"));
        assert!(text.contains("getlastmodified"));
        assert!(text.contains("resourcetype"));
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
}
