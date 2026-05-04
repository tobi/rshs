mod common;

use actix_web::{App, http, test, web};
use common::temp_dir_with_files;
use rshs;
use std::path::PathBuf;

#[actix_web::test]
async fn test_http_get_dir_root() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let body = test::read_body(resp).await;
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("Directory listing for /"));
    assert!(body_str.contains("hello.txt"));
    assert!(body_str.contains("subdir/"));
    assert!(!body_str.contains("../"));
}

#[actix_web::test]
async fn test_http_get_file_content() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/hello.txt").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain")
    );

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[actix_web::test]
async fn test_http_head_file() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(http::Method::HEAD)
        .uri("/hello.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert!(resp.headers().contains_key("content-length"));

    let body = test::read_body(resp).await;
    assert!(body.is_empty());
}

#[actix_web::test]
async fn test_http_head_dir() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(http::Method::HEAD)
        .uri("/")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert!(resp.headers().contains_key("content-length"));

    let body = test::read_body(resp).await;
    assert!(body.is_empty());
}

#[actix_web::test]
async fn test_http_not_found() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/nonexistent.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn test_http_method_not_allowed() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::post().uri("/hello.txt").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);

    let req = test::TestRequest::default()
        .method(http::Method::PUT)
        .uri("/hello.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);

    let req = test::TestRequest::default()
        .method(http::Method::DELETE)
        .uri("/hello.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);
}

#[actix_web::test]
async fn test_http_nested_file() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/subdir/nested.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"Nested file");
}

#[actix_web::test]
async fn test_http_subdir_listing() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/subdir/").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("Directory listing for /subdir/"));
    assert!(body_str.contains("nested.txt"));
    assert!(body_str.contains("../"));
}

#[actix_web::test]
async fn test_http_path_traversal_blocked() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/../outside.txt").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn test_http_mime_type_guess() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/hello.txt").to_request();
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
async fn test_http_dir_listing_sizes() {
    let dir = temp_dir_with_files();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(PathBuf::from(dir.path())))
            .default_service(web::to(rshs::http_server::handle)),
    )
    .await;

    let req = test::TestRequest::get().uri("/").to_request();
    let resp = test::call_service(&app, req).await;
    let body = test::read_body(resp).await;
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    assert!(body_str.contains("13 B"));
}
