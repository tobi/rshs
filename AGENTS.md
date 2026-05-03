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
- tests in `tests/` directory, using `rshs_lib` for test utilities
- Dependencies: `actix-web` (HTTP server), `clap` (CLI args), `tokio` (async runtime, full features), `dav-server` (WebDAV support)
- Edition 2024 — requires Rust 1.85+

## Conventions

- Standard Rust conventions; no custom formatter or lint config overrides
- Run `cargo fmt` then `cargo clippy` before committing

# Testing
- Unit tests in `src/` modules, run with `cargo test`
- Integration tests in `tests/` directory, also run with `cargo test`
- Use `#[cfg(test)]` for test-only code in library crate
- Test coverage is not enforced but encouraged for critical code paths
