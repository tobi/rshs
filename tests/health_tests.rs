mod common;

use std::sync::Arc;

use axum::{Router, body::Body, extract::Request};
use common::temp_dir_with_files;
use tower::ServiceExt;

use rshs::{self, AppState};

fn make_app(dir: &tempfile::TempDir) -> Router {
    let handler = rshs::handlers::webdav::create_dav_handler(dir.path());
    let path = dir.path().to_path_buf();
    Router::new()
        .fallback(rshs::handlers::file::handle)
        .layer(rshs::middleware::health::HealthCheck)
        .with_state(Arc::new(AppState {
            root_dir: path.clone(),
            root_canonical: path.canonicalize().unwrap_or(path),
            dav_handler: handler,
            auth_config: Arc::new(rshs::AuthConfig::new()),
        }))
}

#[tokio::test]
async fn test_health_check_returns_ok() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/")
        .header("x-health-check", "true")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"OK");
}

#[tokio::test]
async fn test_health_check_content_type() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/")
        .header("x-health-check", "true")
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
async fn test_health_check_without_header_passes_through() {
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
async fn test_health_check_with_wrong_header_value_passes_through() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri("/hello.txt")
        .header("x-health-check", "false")
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
async fn test_health_check_with_head_method() {
    let dir = temp_dir_with_files();
    let app = make_app(&dir);

    let req = Request::builder()
        .method(axum::http::Method::HEAD)
        .uri("/")
        .header("x-health-check", "true")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn test_is_health_check_function() {
    use axum::http::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    assert!(!rshs::middleware::health::is_health_check(&headers));

    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("true"),
    );
    assert!(rshs::middleware::health::is_health_check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("false"),
    );
    assert!(!rshs::middleware::health::is_health_check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("1"),
    );
    assert!(!rshs::middleware::health::is_health_check(&headers));
}
