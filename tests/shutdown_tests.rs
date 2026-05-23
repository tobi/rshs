#[cfg(unix)]
mod unix_tests {
    use std::io::Read;
    use std::process::Command;
    use std::time::Duration;

    #[test]
    fn test_graceful_shutdown_on_sigint() {
        let tmpdir = tempfile::TempDir::new().unwrap();
        let serve_dir = tmpdir.path().join("serve");
        std::fs::create_dir(&serve_dir).unwrap();

        let out_file = std::fs::File::create(tmpdir.path().join("stdout.txt")).unwrap();
        let err_file = std::fs::File::create(tmpdir.path().join("stderr.txt")).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_rshs"))
            .arg(&serve_dir)
            .stdout(out_file)
            .stderr(err_file)
            .spawn()
            .expect("failed to spawn server");

        // Give the server time to start up.
        std::thread::sleep(Duration::from_millis(800));

        // Send SIGINT.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }

        let status = child.wait().unwrap();

        let mut stderr = String::new();
        std::fs::File::open(tmpdir.path().join("stderr.txt"))
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();

        assert!(
            status.success(),
            "server exited with code: {:?}, stderr: {stderr}",
            status.code()
        );
        assert!(
            stderr.contains("shutting down gracefully"),
            "stderr should contain shutdown message, got: {stderr}"
        );
    }
}
