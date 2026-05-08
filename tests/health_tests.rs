mod common;

use actix_web::{http, test, web};
use common::temp_dir_with_files;
use rshs;
use std::path::PathBuf;

#[actix_web::test]
async fn test_health_check_returns_ok() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        actix_web::App::new()
            .wrap(rshs::middleware::health_check::HealthCheck)
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/")
        .insert_header(("x-health-check", "true"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"OK");
}

#[actix_web::test]
async fn test_health_check_content_type() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        actix_web::App::new()
            .wrap(rshs::middleware::health_check::HealthCheck)
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/")
        .insert_header(("x-health-check", "true"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain")
    );
}

#[actix_web::test]
async fn test_health_check_without_header_passes_through() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        actix_web::App::new()
            .wrap(rshs::middleware::health_check::HealthCheck)
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/hello.txt").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[actix_web::test]
async fn test_health_check_with_wrong_header_value_passes_through() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        actix_web::App::new()
            .wrap(rshs::middleware::health_check::HealthCheck)
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/hello.txt")
        .insert_header(("x-health-check", "false"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[actix_web::test]
async fn test_health_check_with_head_method() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        actix_web::App::new()
            .wrap(rshs::middleware::health_check::HealthCheck)
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(http::Method::HEAD)
        .uri("/")
        .insert_header(("x-health-check", "true"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);
}

#[actix_web::test]
async fn test_is_health_check_function() {
    use actix_web::http::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    assert!(!rshs::middleware::health_check::is_health_check(&headers));

    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("true"),
    );
    assert!(rshs::middleware::health_check::is_health_check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("false"),
    );
    assert!(!rshs::middleware::health_check::is_health_check(&headers));

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-health-check"),
        HeaderValue::from_static("1"),
    );
    assert!(!rshs::middleware::health_check::is_health_check(&headers));
}
