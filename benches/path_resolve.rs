mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::TempDir;
use tower::ServiceExt;

use common::*;

fn bench_resolve_write_target(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let body = vec![b'a'; 1024];

    let mut group = c.benchmark_group("path/resolve_via_router");

    // Shallow path — PUT with a single-level path exercises resolve_and_guard
    group.bench_function("put_shallow_1_level", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_put("/newfile.txt", &body)).await;
            });
        });
    });

    // Deep nested path — PUT with a 5-level deep path
    group.bench_function("put_deep_5_levels", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let deep = dir.path().join("a/b/c/d/e");
            std::fs::create_dir_all(&deep).unwrap();
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router
                    .oneshot(make_put("/a/b/c/d/e/nested.txt", &body))
                    .await;
            });
        });
    });

    // Percent-encoded path
    group.bench_function("put_percent_encoded", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_put("/file%20name.txt", &body)).await;
            });
        });
    });

    group.finish();
}

fn bench_resolve_existing_via_get(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("path/resolve_existing_via_get");

    // Shallow: 1 level
    group.bench_function("shallow_1_level", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_file(dir.path(), "file.txt", 1024);
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_get("/file.txt")).await;
            });
        });
    });

    // Deep: 5 levels
    group.bench_function("deep_5_levels", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let deep = dir.path().join("a/b/c/d/e");
            std::fs::create_dir_all(&deep).unwrap();
            create_file(&deep, "nested.txt", 1024);
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_get("/a/b/c/d/e/nested.txt")).await;
            });
        });
    });

    // UTF-8 percent-encoded path
    group.bench_function("utf8_encoded", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let name = "\u{00E9}l\u{00E8}ve.txt";
            create_file(dir.path(), name, 1024);
            let router = bench_router(dir.path());
            let encoded = "/%C3%A9l%C3%A8ve.txt";
            rt.block_on(async {
                let _ = router.oneshot(make_get(encoded)).await;
            });
        });
    });

    group.finish();
}

fn bench_cold_vs_hot_cache(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("path/cache");

    // Cold: each iteration creates a new TempDir (fresh filesystem)
    group.bench_function("get_cold_new_tempdir", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_file(dir.path(), "file.txt", 1024);
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_get("/file.txt")).await;
            });
        });
    });

    // Hot cache: same router and file across iterations
    let dir = TempDir::new().unwrap();
    create_files(dir.path(), 100, 65536);
    let router = bench_router(dir.path());

    group.bench_function("get_hot_reuse", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/file_00000.bin")).await;
            });
        });
    });

    group.finish();
}

criterion_group!(
    path_resolve,
    bench_resolve_write_target,
    bench_resolve_existing_via_get,
    bench_cold_vs_hot_cache,
);

criterion_main!(path_resolve);
