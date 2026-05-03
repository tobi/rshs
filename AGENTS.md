# AGENTS.md

## Build & Run

```sh
cargo build
cargo run
cargo test
cargo fmt
cargo clippy
```

## Architecture

- binary crate (`rshs`), entrypoint: `src/main.rs`
- library crate (`rshs_lib`), entrypoint: `src/lib.rs`
- Dependencies: `actix-web` (HTTP server), `clap` (CLI args), `tokio` (async runtime, full features), `webdav-handler` (WebDAV support)
- Edition 2024 — requires Rust 1.85+

## Conventions

- Standard Rust conventions; no custom formatter or lint config overrides
- Run `cargo fmt` then `cargo clippy` before committing
