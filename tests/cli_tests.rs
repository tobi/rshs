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
fn test_cli_missing_root_dir() {
    let result = Cli::try_parse_from(["rshs"]);
    assert!(result.is_err());
}
