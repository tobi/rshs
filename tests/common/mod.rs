use std::io::Write;

pub fn temp_dir_with_files() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("failed to create temp dir");
    let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
    f.write_all(b"Hello, World!").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let mut f = std::fs::File::create(dir.path().join("subdir/nested.txt")).unwrap();
    f.write_all(b"Nested file").unwrap();
    dir
}
