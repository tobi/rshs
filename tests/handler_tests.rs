use rshs::{AuthConfig, ServerConfig};

#[test]
fn test_server_config_new() {
    let config = ServerConfig::new(
        std::path::PathBuf::from("/tmp/test"),
        "127.0.0.1".into(),
        3000,
        None,
        AuthConfig::new(),
    );
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.root_dir, std::path::PathBuf::from("/tmp/test"));
}
