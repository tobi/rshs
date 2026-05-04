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

- Single crate `rshs` with both binary (`src/main.rs`) and library (`src/lib.rs`) targets
- Library root `src/lib.rs` declares modules and re-exports public API
- Tests live in `tests/` directory (integration tests)
- Edition 2024 — requires Rust 1.85+

### Module Map

| Module                | Path                        | Purpose                                                      |
| --------------------- | --------------------------- | ------------------------------------------------------------ |
| `cli`                 | `src/cli/mod.rs`            | CLI argument parsing (clap derive)                           |
| `server`              | `src/server/mod.rs`         | Server orchestration, `ServerConfig`, conditional middleware |
| `server::auth_basic`  | `src/server/auth_basic.rs`  | Basic Auth credential store and validator                    |
| `server::webdav`      | `src/server/webdav.rs`      | WebDAV handler (local FS + fake locks)                       |
| `server::http_server` | `src/server/http_server.rs` | A read-only file server accessible via a browser             |

### Dependencies

| Crate                    | Features        | Purpose                    |
| ------------------------ | --------------- | -------------------------- |
| `actix-web` 4.13         | —               | HTTP server framework      |
| `actix-web-httpauth` 0.8 | —               | Basic Auth middleware      |
| `clap` 4.6               | `derive`, `env` | CLI args + env var support |
| `tokio` 1.52             | `full`          | Async runtime              |
| `dav-server` 0.11        | `actix-compat`  | WebDAV protocol handling   |
| `mime_guess` 2           | —               | MIME type detection        |
| `log` 0.4                | —               | Logging facade             |
| `env_logger` 0.11        | —               | Logging backend            |

### Key Patterns

- **Clone-before-move**: Clone values before `move ||` closures to capture owned data
- **Conditional middleware**: In actix-web, `App::wrap()` changes the concrete `ServiceFactory` type, so conditional middleware requires separate `HttpServer::new()` paths in `if`/`else` branches — do NOT reassign `App` to the same variable
- **App data**: Shared state (`DavHandler`, `AuthConfig`) passed via `web::Data<T>`
- **Auth**: `AuthConfig` holds `HashMap<String, String>`. When users are configured, Basic Auth middleware is applied globally. When empty, the server runs without authentication

## Conventions

- Standard Rust conventions; no custom formatter or lint config overrides
- Run `cargo fmt` then `cargo clippy` before committing — both must produce zero warnings
- All public types are re-exported from `src/lib.rs`; tests import from `rshs` crate root

## Testing

- Unit tests in `src/` modules, integration tests in `tests/` — all run with `cargo test`
- External crates in tests reference via the `rshs` crate (not by relative module paths)
- Use `#[cfg(test)]` for test-only code in the library crate
- Add or update tests for the code you change, even if nobody asked

## Authentication

Basic HTTP Authentication (RFC 7617) is supported via `--user` / `-U` and `RSHS_USERS` env var.

```sh
rshs --user admin:secret --user viewer:public ./docs
RSHS_USERS="admin:secret;viewer:public" rshs ./docs
```

- Credentials format: `username:password`, multiple pairs separated by `;`
- CLI values take precedence over env var values for the same username
- If no users are configured, the server runs without authentication (backward compatible)

## Modes

The server always runs in HTTP + WebDAV hybrid mode:

```sh
rshs ./docs               # Serve files in ./docs
rshs                       # Default: serve current directory
RSHS_ROOT_DIR=./docs rshs  # Set root via env var
```

- **Browser**: GET/HEAD → HTML directory listing, file serving
- **WebDAV client**: PROPFIND/PUT/DELETE/MKCOL… → WebDAV protocol

# Environment Variables

| Env Var         | Description                   |
| --------------- | ----------------------------- |
| `RSHS_ROOT_DIR` | Root directory (default: `.`) |
| `RSHS_HOST`     | Bind address                  |
| `RSHS_PORT`     | Bind port                     |
| `RSHS_USERS`    | Basic Auth credentials        |
