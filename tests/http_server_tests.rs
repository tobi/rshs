mod common;

use std::sync::Arc;

use axum::{Router, body::Body, extract::Request};
use common::temp_dir_with_files;
use tower::ServiceExt;

use rshs::{self, AppState};

fn make_app(dir: &tempfile::TempDir) -> Router {
    let handler = rshs::webdav::create_dav_handler(dir.path());
    Router::new()
        .fallback(rshs::file::handle)
        .with_state(Arc::new(AppState {
            root_dir: Arc::new(dir.path().to_path_buf()),
            dav_handler: Arc::new(handler),
            auth_config: Arc::new(rshs::AuthConfig::new()),
        }))
}

#[tokio::test]
async fn test_http_get_dir_root() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("Index of /"));
    assert!(body_str.contains("hello.txt"));
    assert!(body_str.contains("subdir/"));
    assert!(!body_str.contains("../"));
}

#[tokio::test]
async fn test_http_get_file_content() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain")
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[tokio::test]
async fn test_http_head_file() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::HEAD)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert!(resp.headers().contains_key("content-length"));

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn test_http_head_dir() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::HEAD)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());
    assert!(resp.headers().contains_key("content-length"));

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn test_http_not_found() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/nonexistent.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_http_method_not_allowed() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::POST)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);

    let req = Request::builder()
        .method(axum::http::Method::PUT)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);

    let req = Request::builder()
        .method(axum::http::Method::DELETE)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_http_nested_file() {
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

#[tokio::test]
async fn test_http_subdir_listing() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/subdir/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_success());

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("Index of /subdir/"));
    assert!(body_str.contains("nested.txt"));
    assert!(body_str.contains("../"));
}

#[tokio::test]
async fn test_http_path_traversal_blocked() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/../outside.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_http_mime_type_guess() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/hello.txt")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain")
    );
}

#[tokio::test]
async fn test_http_dir_listing_sizes() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        body_str
            .lines()
            .any(|l| l.contains("hello.txt") && l.ends_with("13"))
    );
    assert!(
        body_str
            .lines()
            .any(|l| l.contains("subdir/") && l.contains("-"))
    );
}
