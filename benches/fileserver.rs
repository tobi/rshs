mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use tempfile::TempDir;
use tower::ServiceExt;

use common::*;

fn bench_get_file(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    create_file(dir.path(), "tiny.txt", 13);
    create_file(dir.path(), "small.txt", 1024);
    create_file(dir.path(), "medium.bin", 65536);
    create_file(dir.path(), "large.bin", 1024 * 1024);
    create_file(dir.path(), "xlarge.bin", 1024 * 1024 * 10);

    let router = bench_router(dir.path());
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/GET");

    group.throughput(Throughput::Bytes(13));
    group.bench_function("tiny_13b", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/tiny.txt")).await;
            });
        });
    });

    group.throughput(Throughput::Bytes(1024));
    group.bench_function("small_1kb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/small.txt")).await;
            });
        });
    });

    group.throughput(Throughput::Bytes(65536));
    group.bench_function("medium_64kb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/medium.bin")).await;
            });
        });
    });

    group.throughput(Throughput::Bytes(1024 * 1024));
    group.bench_function("large_1mb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/large.bin")).await;
            });
        });
    });

    group.throughput(Throughput::Bytes(1024 * 1024 * 10));
    group.bench_function("xlarge_10mb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_get("/xlarge.bin")).await;
            });
        });
    });

    group.finish();
}

fn bench_head_file(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    create_file(dir.path(), "hello.txt", 1024);

    let router = bench_router(dir.path());
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/HEAD");
    group.bench_function("small_file", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_head("/hello.txt")).await;
            });
        });
    });
    group.finish();
}

fn bench_dir_listing(c: &mut Criterion) {
    let mut group = c.benchmark_group("fileserver/dir_listing");

    for count in [10u32, 50, 200, 1000] {
        let dir = TempDir::new().unwrap();
        create_small_files(dir.path(), count as usize);
        let router = bench_router(dir.path());
        let rt = tokio::runtime::Runtime::new().unwrap();

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let _ = router.clone().oneshot(make_get("/")).await;
                });
            });
        });
    }

    group.finish();
}

fn bench_put(c: &mut Criterion) {
    let body_1kb = vec![b'x'; 1024];
    let body_10mb = vec![b'x'; 1024 * 1024 * 10];

    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/PUT");

    // PUT create new (create_new succeeds)
    group.throughput(Throughput::Bytes(1024));
    group.bench_function("create_new_1kb", |b| {
        let dir = TempDir::new().unwrap();
        let router = bench_router(dir.path());
        b.iter(|| {
            let name = unique_name("put_new");
            rt.block_on(async {
                let _ = router
                    .clone()
                    .oneshot(make_put(&format!("/{name}"), &body_1kb))
                    .await;
            });
        });
    });

    // PUT overwrite (create_new fails → create)
    group.throughput(Throughput::Bytes(1024));
    group.bench_function("overwrite_1kb", |b| {
        let dir = TempDir::new().unwrap();
        let fname = "overwrite_target.bin";
        create_file(dir.path(), fname, 512);
        let router = bench_router(dir.path());
        b.iter(|| {
            rt.block_on(async {
                let _ = router
                    .clone()
                    .oneshot(make_put(&format!("/{fname}"), &body_1kb))
                    .await;
            });
        });
    });

    // PUT large file
    group.throughput(Throughput::Bytes(1024 * 1024 * 10));
    group.bench_function("large_10mb", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router
                    .clone()
                    .oneshot(make_put("/put_large.bin", &body_10mb))
                    .await;
            });
        });
    });

    group.finish();
}

fn bench_delete_file(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/DELETE");
    group.bench_function("small_file", |b| {
        b.iter(|| {
            let dir = TempDir::new().unwrap();
            create_file(dir.path(), "to_delete.txt", 1024);
            let router = bench_router(dir.path());
            rt.block_on(async {
                let _ = router.oneshot(make_delete("/to_delete.txt")).await;
            });
        });
    });
    group.finish();
}

fn bench_delete_dir_tree(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/DELETE_dir_tree");

    for depth in [2u32, 3, 5] {
        group.throughput(Throughput::Elements(depth as u64 * 10));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter(|| {
                let dir = TempDir::new().unwrap();
                create_nested_tree(dir.path(), depth as usize, 10);
                let router = bench_router(dir.path());
                rt.block_on(async {
                    let _ = router.oneshot(make_delete("/level_0")).await;
                });
            });
        });
    }

    group.finish();
}

fn bench_options(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let router = bench_router(dir.path());
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fileserver/OPTIONS");
    group.bench_function("static_response", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = router.clone().oneshot(make_options("/")).await;
            });
        });
    });
    group.finish();
}

criterion_group!(
    fileserver,
    bench_get_file,
    bench_head_file,
    bench_dir_listing,
    bench_put,
    bench_delete_file,
    bench_delete_dir_tree,
    bench_options,
);

criterion_main!(fileserver);
