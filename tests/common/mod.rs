use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use rshs::{AppState, AuthConfig, make_router};

pub fn temp_dir_with_files() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("failed to create temp dir");
    let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
    f.write_all(b"Hello, World!").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let mut f = std::fs::File::create(dir.path().join("subdir/nested.txt")).unwrap();
    f.write_all(b"Nested file").unwrap();
    dir
}

pub fn make_test_router(root_dir: &Path, auth: AuthConfig) -> Router {
    let state = Arc::new(AppState::new(root_dir.to_path_buf(), auth));
    make_router(state)
}
