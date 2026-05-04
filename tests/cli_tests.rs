use clap::Parser;
use rshs::Cli;

#[test]
fn test_cli_default_values() {
    let cli = Cli::try_parse_from(["rshs", "/tmp/test"]).unwrap();
    assert_eq!(cli.root_dir, "/tmp/test");
    assert_eq!(cli.host, "0.0.0.0");
    assert_eq!(cli.port, 8080);
}

#[test]
fn test_cli_custom_host_short() {
    let cli = Cli::try_parse_from(["rshs", "-H", "127.0.0.1", "/tmp/test"]).unwrap();
    assert_eq!(cli.host, "127.0.0.1");
}

#[test]
fn test_cli_custom_host_long() {
    let cli = Cli::try_parse_from(["rshs", "--host", "127.0.0.1", "/tmp/test"]).unwrap();
    assert_eq!(cli.host, "127.0.0.1");
}

#[test]
fn test_cli_custom_port_short() {
    let cli = Cli::try_parse_from(["rshs", "-p", "3000", "/tmp/test"]).unwrap();
    assert_eq!(cli.port, 3000);
}

#[test]
fn test_cli_custom_port_long() {
    let cli = Cli::try_parse_from(["rshs", "--port", "3000", "/tmp/test"]).unwrap();
    assert_eq!(cli.port, 3000);
}

#[test]
fn test_cli_full() {
    let cli = Cli::try_parse_from([
        "rshs",
        "--host",
        "127.0.0.1",
        "--port",
        "9090",
        "/srv/webdav",
    ])
    .unwrap();
    assert_eq!(cli.root_dir, "/srv/webdav");
    assert_eq!(cli.host, "127.0.0.1");
    assert_eq!(cli.port, 9090);
}

#[test]
fn test_cli_root_dir_default() {
    let cli = Cli::try_parse_from(["rshs"]).unwrap();
    assert_eq!(cli.root_dir, ".");
}

#[test]
fn test_cli_root_dir_custom() {
    let cli = Cli::try_parse_from(["rshs", "/custom/path"]).unwrap();
    assert_eq!(cli.root_dir, "/custom/path");
}

#[test]
fn test_cli_user_single() {
    let cli = Cli::try_parse_from(["rshs", "--user", "admin:secret", "/tmp/test"]).unwrap();
    assert_eq!(cli.users.len(), 1);
    assert_eq!(cli.users[0], "admin:secret");
}

#[test]
fn test_cli_user_multiple() {
    let cli = Cli::try_parse_from([
        "rshs",
        "--user",
        "admin:secret",
        "--user",
        "viewer:public",
        "/tmp/test",
    ])
    .unwrap();
    assert_eq!(cli.users.len(), 2);
    assert_eq!(cli.users[0], "admin:secret");
    assert_eq!(cli.users[1], "viewer:public");
}

#[test]
fn test_cli_to_auth_config() {
    let cli = Cli::try_parse_from([
        "rshs",
        "--user",
        "alice:pass1",
        "--user",
        "bob:pass2",
        "/tmp/test",
    ])
    .unwrap();
    let auth = cli.to_auth_config();
    assert!(!auth.is_empty());
    assert!(auth.validate("alice", "pass1"));
    assert!(auth.validate("bob", "pass2"));
    assert!(!auth.validate("alice", "wrong"));
    assert!(!auth.validate("eve", "pass1"));
}

#[test]
fn test_cli_to_auth_config_empty() {
    let cli = Cli::try_parse_from(["rshs", "/tmp/test"]).unwrap();
    let auth = cli.to_auth_config();
    assert!(auth.is_empty());
}

#[test]
fn test_cli_to_auth_config_skips_malformed() {
    let cli = Cli::try_parse_from(["rshs", "--user", "bob", "--user", "alice:pass", "/tmp/test"])
        .unwrap();
    let auth = cli.to_auth_config();
    assert_eq!(auth.validate("alice", "pass"), true);
    assert_eq!(auth.is_empty(), false);
}

#[test]
fn test_cli_to_auth_config_skips_empty_username() {
    let cli = Cli::try_parse_from(["rshs", "--user", ":password", "/tmp/test"]).unwrap();
    let auth = cli.to_auth_config();
    assert!(auth.is_empty());
}

#[test]
fn test_cli_combined_flags() {
    let cli = Cli::try_parse_from([
        "rshs",
        "--user",
        "u:p",
        "-H",
        "127.0.0.1",
        "-p",
        "9999",
        "/data",
    ])
    .unwrap();
    assert_eq!(cli.host, "127.0.0.1");
    assert_eq!(cli.port, 9999);
    assert_eq!(cli.root_dir, "/data");
    assert_eq!(cli.users.len(), 1);
}
