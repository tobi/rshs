mod common;

use actix_web::{App, test, web};
use common::temp_dir_with_files;
use rshs;

#[actix_web::test]
async fn test_server_config_new() {
    let config = rshs::ServerConfig::new(
        "127.0.0.1".into(),
        3000,
        std::path::PathBuf::from("/tmp/test"),
    );
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.root_dir, std::path::PathBuf::from("/tmp/test"));
}

#[actix_web::test]
async fn test_get_root_returns_405() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let req = test::TestRequest::get().uri("/").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 405);
}

#[actix_web::test]
async fn test_get_file_content() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let req = test::TestRequest::get().uri("/hello.txt").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    assert_eq!(body.as_ref(), b"Hello, World!");
}

#[actix_web::test]
async fn test_head_file() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(actix_web::http::Method::HEAD)
        .uri("/hello.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[actix_web::test]
async fn test_options_request() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(actix_web::http::Method::OPTIONS)
        .uri("/")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[actix_web::test]
async fn test_propfind_request() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let method = actix_web::http::Method::from_bytes(b"PROPFIND").unwrap();
    let req = test::TestRequest::default()
        .method(method)
        .uri("/")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    assert!(body_str.contains("hello.txt"));
    assert!(body_str.contains("subdir"));
}

#[actix_web::test]
async fn test_not_found() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/nonexistent.txt")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404);
}

#[actix_web::test]
async fn test_nested_file() {
    let dir = temp_dir_with_files();
    let handler = rshs::dav::create_dav_handler(dir.path());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(handler))
            .default_service(web::to(rshs::dav::dav_route)),
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
