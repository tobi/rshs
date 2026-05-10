mod common;

use std::sync::Arc;

use axum::{Router, body::Body, extract::Request};
use common::temp_dir_with_files;
use tower::ServiceExt;

use rshs::{self, AppState};

fn make_app(dir: &tempfile::TempDir) -> Router {
    let handler = rshs::handlers::webdav::create_dav_handler(dir.path());
    let path = Arc::new(dir.path().to_path_buf());
    Router::new()
        .fallback(rshs::handlers::webdav::dav_route)
        .with_state(Arc::new(AppState {
            root_dir: path.clone(),
            root_canonical: path.canonicalize().map(Arc::new).unwrap_or_else(|_| path),
            dav_handler: Arc::new(handler),
            auth_config: Arc::new(rshs::AuthConfig::new()),
        }))
}

#[tokio::test]
async fn test_server_config_new() {
    let config = rshs::ServerConfig::new(
        std::path::PathBuf::from("/tmp/test"),
        "127.0.0.1".into(),
        3000,
        None,
        rshs::AuthConfig::new(),
    );
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.root_dir, std::path::PathBuf::from("/tmp/test"));
}

#[tokio::test]
async fn test_get_root_returns_405() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 405);
}

#[tokio::test]
async fn test_get_file_content() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[tokio::test]
async fn test_head_file() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::HEAD)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
}

#[tokio::test]
async fn test_options_request() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::OPTIONS)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
}

#[tokio::test]
async fn test_propfind_request() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::from_bytes(b"PROPFIND").unwrap())
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("hello.txt"));
    assert!(body_str.contains("subdir"));
}

#[tokio::test]
async fn test_not_found() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/nonexistent.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn test_nested_file() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/subdir/nested.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"Nested file");
}
