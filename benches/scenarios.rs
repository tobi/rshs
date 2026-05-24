mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::TempDir;
use tower::ServiceExt;

use axum::extract::Request;

use common::*;

fn bench_browser_browse(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("scenarios");

    group.bench_function("browser_browse_sequence", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_file(dir.path(), "index.html", 2048);
            create_file(dir.path(), "style.css", 512);
            create_dir(dir.path(), "images");
            create_file(dir.path(), "images/logo.png", 8192);
            create_dir(dir.path(), "docs");
            create_file(dir.path(), "docs/readme.txt", 1024);
            let router = bench_router(dir.path());

            rt.block_on(async move {
                let _ = router.clone().oneshot(make_get("/")).await;
                let _ = router.clone().oneshot(make_get("/images/")).await;
                let _ = router.clone().oneshot(make_get("/images/logo.png")).await;
            });
        });
    });

    group.bench_function("webdav_sync_sequence", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let files = 20;
            create_small_files(dir.path(), files);
            let router = bench_router(dir.path());

            rt.block_on(async move {
                let propfind_body =
                    br#"<?xml version="1.0"?><D:propfind xmlns:D="DAV:"><D:allprop/></D:propfind>"#;
                let _ = router
                    .clone()
                    .oneshot(make_propfind("/", "1", propfind_body))
                    .await;
                for i in [0usize, 3, 7, 12, 17] {
                    let _ = router
                        .clone()
                        .oneshot(make_get(&format!("/file_{i:05}.txt")))
                        .await;
                }
            });
        });
    });

    group.bench_function("lock_edit_unlock_sequence", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_file(dir.path(), "target.txt", 1024);
            let router = bench_router(dir.path());
            let body_1kb = vec![b'x'; 1024];

            rt.block_on(async move {
                let lock_req = make_lock("/target.txt", &lock_body_exclusive());
                let lock_resp = router.clone().oneshot(lock_req).await.unwrap();
                let token = lock_resp
                    .headers()
                    .get("lock-token")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.trim_matches('<').trim_matches('>').to_string())
                    .unwrap_or_default();

                let put_req = Request::builder()
                    .method(axum::http::Method::PUT)
                    .uri("/target.txt")
                    .header("if", format!("(<{token}>)"))
                    .body(axum::body::Body::from(body_1kb))
                    .unwrap();
                let _ = router.clone().oneshot(put_req).await;

                let _ = router.oneshot(make_unlock("/target.txt", &token)).await;
            });
        });
    });

    // Mixed workload on a populated dir
    group.bench_function("mixed_workload", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_small_files(dir.path(), 30);
            create_dir(dir.path(), "media");
            create_file(dir.path(), "media/big.bin", 65536);
            let router = bench_router(dir.path());

            rt.block_on(async move {
                let _ = router.clone().oneshot(make_get("/file_00000.txt")).await;
                let _ = router.clone().oneshot(make_get("/file_00001.txt")).await;
                let _ = router.clone().oneshot(make_get("/file_00002.txt")).await;
                let _ = router.clone().oneshot(make_get("/media/big.bin")).await;
                let _ = router.clone().oneshot(make_get("/file_00003.txt")).await;

                let propfind_body =
                    br#"<?xml version="1.0"?><D:propfind xmlns:D="DAV:"><D:allprop/></D:propfind>"#;
                let _ = router
                    .clone()
                    .oneshot(make_propfind("/media/", "1", propfind_body))
                    .await;

                let _ = router
                    .clone()
                    .oneshot(make_put("/file_new.txt", b"new content"))
                    .await;

                let _ = router.clone().oneshot(make_options("/")).await;
            });
        });
    });

    group.finish();
}

criterion_group!(scenarios, bench_browser_browse);

criterion_main!(scenarios);
