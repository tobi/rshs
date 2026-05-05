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
| `server::shadow`      | `src/server/shadow.rs`      | Shadow file management (create, load, merge, write)          |

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
| `sha-crypt` 0.4          | —               | SHA-512 crypt hash verification |

### Key Patterns

- **Clone-before-move**: Clone values before `move ||` closures to capture owned data
- **Conditional middleware**: In actix-web, `App::wrap()` changes the concrete `ServiceFactory` type, so conditional middleware requires separate `HttpServer::new()` paths in `if`/`else` branches — do NOT reassign `App` to the same variable
- **App data**: Shared state (`DavHandler`, `AuthConfig`) passed via `web::Data<T>`
- **Auth**: `AuthConfig` holds `HashMap<String, Credential>`. When users are configured, Basic Auth middleware is applied globally. When empty, the server runs without authentication
- **Shadow file**: Persistent credential store (`username:$hash$...` format). CLI credentials (`--user`) can be merged in and optionally written back to disk

## Conventions

- Standard Rust conventions; no custom formatter or lint config overrides
- Run `cargo fmt` then `cargo clippy` before committing — both must produce zero warnings
- All public types are re-exported from `src/lib.rs`; tests import from `rshs` crate root
- Update `AGENTS.md`, `README.md` and `docs/` accordingly when new features are added or existing ones are changed

## Testing

- Unit tests in `src/` modules, integration tests in `tests/` — all run with `cargo test`
- External crates in tests reference via the `rshs` crate (not by relative module paths)
- Use `#[cfg(test)]` for test-only code in the library crate
- Add or update tests for the code you change, even if nobody asked

## Authentication

Basic HTTP Authentication (RFC 7617) is supported via `--user` / `-u` and `RSHS_USERS` env var.

```sh
rshs --user admin:secret --user viewer:public ./docs
RSHS_USERS="admin:secret;viewer:public" rshs ./docs
```

- Credentials format: `username:password`, multiple pairs separated by `;`
- CLI values take precedence over env var values for the same username
- If no users are configured, the server runs without authentication (backward compatible)

Shadow files provide persistent credential storage in SHA-512 crypt format:

```sh
rshs -S ./shadow --user admin:secret ./docs
rshs -S /etc/rshs/shadow:rw -W --user admin:newpass ./docs
RSHS_SHADOW_FILE=./shadow:ro rshs ./docs
```

- Shadow file path can be suffixed with `:rw` (default) or `:ro` to control write access
- `-W` / `--shadow-write` writes CLI credentials into the shadow file after merge
- Shadow files store passwords hashed with SHA-512 crypt (`$6$...`)

## Modes

The server always runs in HTTP + WebDAV hybrid mode:

```sh
rshs ./docs               # Serve files in ./docs
rshs                       # Default: serve current directory
RSHS_ROOT_DIR=./docs rshs  # Set root via env var
```

- **Browser**: GET/HEAD → HTML directory listing, file serving
- **WebDAV client**: PROPFIND/PUT/DELETE/MKCOL… → WebDAV protocol

## Logging

Log level is determined by the following priority (highest first):

1. `-q` / `--quiet` — suppress all logs (`off`)
2. `-vv` / `--verbose --verbose` — trace level
3. `-v` / `--verbose` — debug level
4. `RSHS_LOG` env var — arbitrary filter string (e.g. `info`, `rshs=debug`)
5. Default — `info` level

```sh
rshs -v                  # debug
rshs -vv                 # trace
rshs -q                  # silent
rshs                     # info (or RSHS_LOG if set)
```

`RSHS_LOG_STYLE` controls log output style (`auto`, `always`, `never`).

# Environment Variables

| Env Var          | Description                                       |
| ---------------- | ------------------------------------------------- |
| `RSHS_ROOT_DIR`  | Root directory (default: `.`)                     |
| `RSHS_HOST`      | Bind address                                      |
| `RSHS_PORT`      | Bind port                                         |
| `RSHS_USERS`     | Basic Auth credentials                            |
| `RSHS_LOG`       | Logging level (e.g. `info`)                       |
| `RSHS_LOG_STYLE` | Log output style (e.g. `auto`, `always`, `never`) |
| `RSHS_SHADOW_FILE` | Shadow file path with optional `:rw`/`:ro` suffix |
