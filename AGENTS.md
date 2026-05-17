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

```
src/
  main.rs                       # Entry point: CLI parse, logging init, start server
  lib.rs                        # Module declarations, public re-exports

  cli.rs                        # clap-derived CLI args (Cli, ShadowFileArg)

  auth.rs                       # AuthConfig, Credential, shadow file mgmt, auth middleware

  handlers/
    mod.rs
    http.rs                     # GET/HEAD + PUT/DELETE/OPTIONS (directory listing + file ops)
    webdav.rs                   # PROPFIND/MKCOL/COPY/MOVE/PROPPATCH handler
    locks.rs                    # LOCK/UNLOCK handler

  webdav/
    mod.rs                      # WebDAV Method constants (LazyLock), types, parse helpers
    xml.rs                      # Multistatus XML generation (PROPFIND response)
    fs.rs                       # Filesystem traversal + href encoding

  middleware/
    mod.rs
    health.rs                   # Health check middleware (tower Layer)
    auth.rs                     # Basic Auth middleware (auto-skips when no users configured)
    lock.rs                     # Lock enforcement middleware (tower Layer)

  server/
    mod.rs                      # AppState, ServerConfig, Router construction, serve
    tls.rs                      # TlsConfig (PEM + fingerprint + ALPN), TlsListener

  utils/
    mod.rs
    error.rs                    # OrStatus trait + ok_or_return! macro
    path.rs                     # Path resolution (resolve_existing, resolve_write_target, resolve_and_guard)
    time.rs                     # Calendar formatting for directory listings
```

### Dependencies

| Crate                    | Features                        | Purpose                             |
| ------------------------ | ------------------------------- | ----------------------------------- |
| `axum` 0.8               | `http2`                         | HTTP server framework               |
| `tokio` 1.52             | `rt-multi-thread,net,macros,fs` | Async runtime                       |
| `tower` 0.5              | —                               | Middleware traits (Layer, Service)  |
| `tower-http` 0.6         | `trace`                         | Request tracing middleware          |
| `tokio-rustls` 0.26      | —                               | TLS acceptor for axum               |
| `tokio-util` 0.7         | `io`                            | StreamReader for PUT body streaming |
| `rustls` 0.23            | —                               | TLS protocol implementation         |
| `rustls-pemfile` 2.2     | —                               | PEM certificate/key parsing         |
| `sha2` 0.11              | —                               | Certificate fingerprint             |
| `clap` 4.6               | `derive`, `env`                 | CLI args + env var support          |
| `futures-util` 0.3       | —                               | Stream combinators (TryStreamExt)   |
| `mime_guess` 2           | —                               | MIME type detection                 |
| `percent-encoding` 2     | —                               | URI percent-encode/decode           |
| `quick-xml` 0.40         | —                               | XML parsing + generation (WebDAV)   |
| `base64` 0.22            | —                               | Basic Auth header decoding          |
| `sha-crypt` 0.6          | `getrandom`                     | SHA-512 crypt hash verification     |
| `tracing` 0.1            | —                               | Structured logging facade           |
| `tracing-subscriber` 0.3 | `env-filter`, `fmt`             | Log output + filter engine          |

### Key Patterns

- **App state**: Shared state via `AppState` struct wrapped in `Arc`, accessed by handlers
  via `axum::extract::State<Arc<AppState>>`. Fields: `root_dir` (serve root path),
  `root_canonical` (cached canonical form for path traversal checks), `auth_config`,
  `dead_props` (WebDAV dead property store), `locks` (lock store). Router built with
  `.with_state(Arc::new(state))`.
  `AppState` also provides convenience methods delegates to `utils::path`:
  `state.resolve_existing(path)`, `state.resolve_write_target(path)`,
  `state.resolve_and_guard(path, create_parents)`.
- **File I/O**: Hot-path file operations (GET/HEAD serving, directory listing) use
  `tokio::fs` to offload blocking syscalls from async worker threads onto the blocking
  thread pool. Startup-only I/O (TLS cert/key loading, shadow file reads) uses
  synchronous `std::fs` since it runs before the server accepts connections and does
  not compete for worker threads.
- **Middleware via tower Layer**: Middleware is applied with `Router::layer(L)`. Tower Layers
  compose from inside out — the last `.layer()` in the chain runs first.
- **Middleware order**: `HealthCheck` (outermost) → `LockEnforce` → `Auth` → `TraceLayer` → handler.
  HealthCheck intercepts `x-health-check: true` before auth. Auth middleware auto-skips when
  no users are configured (`auth_config.is_empty()`). LockEnforce checks write operations
  (PUT/DELETE/MKCOL/PROPPATCH) against the lock store before the handler runs.
- **Request dispatch**: `.fallback(any(dispatch))` routes all requests through a single
  `dispatch` function that branches by HTTP method:
  `GET`/`HEAD` → `http::handle_get_head`,
  `PUT` → `http::handle_put`,
  `DELETE` → `http::handle_delete`,
  `OPTIONS` → `http::handle_options`,
  `PROPFIND` → `webdav::handle_propfind`,
  `MKCOL` → `webdav::handle_mkcol`,
  `COPY` → `webdav::handle_copy`,
  `MOVE` → `webdav::handle_move`,
  `PROPPATCH` → `webdav::handle_proppatch`,
  `LOCK` → `locks::handle_lock`,
  `UNLOCK` → `locks::handle_unlock`,
  unknown → `501 Not Implemented`.
- **Path resolution**: `utils::path` provides three functions + one error type:
  - `resolve_existing()` — canonicalize + traversal check for read ops (GET/HEAD) and delete ops (DELETE)
  - `resolve_write_target()` — segment check + traversal guard for write ops (PUT/DELETE/MKCOL)
  - `resolve_and_guard()` — combined: resolve target + create parent dirs (optional) + canonicalize + traversal check
  - `ResolveTargetError` — tagged error type with `InvalidPath`, `ParentCanonicalizeFailed`, `TraversalBlocked`
    All percent-decode the URI path via `percent_encoding::percent_decode_str`.
- **Error handling**: `utils::error::OrStatus` trait extends `Result<T, E: Display>` with
  `.or_400(msg)`, `.or_404(msg)`, `.or_500(msg)`, `.or_status(code, msg)` methods that
  map errors to `Result<T, Response>` with tracing log. `ok_or_return!` macro unwraps
  `Result<T, Response>` or early-returns from the enclosing handler function.
- **XML generation**: `webdav/xml.rs` defines `XmlWriterExt` trait (adds `.ev(event)` to
  `Writer<Cursor<Vec<u8>>>` as shorthand for `.write_event(event).unwrap()`).
  Helper functions: `multistatus(xml)` → `207 Multi-Status`, `xml_response(status, xml)`
  for general XML responses.
- **Lock system**: In-memory lock support via `LockStore` (`Arc<RwLock<HashMap<PathBuf, Vec<LockInfo>>>>`).
  Exclusive write locks only (shared + depth:infinity TODO). Lock enforcement via tower Layer middleware
  (`middleware::lock::lock_enforce`), which intercepts PUT/DELETE/MKCOL/PROPPATCH and rejects with
  `423 Locked` unless the request carries a matching `If` header. Locks are ephemeral (lost on restart).
- **Auth**: `AuthConfig` holds `HashMap<String, Credential>`. Auth middleware is always present
  in the chain but becomes a no-op when `is_empty()`. 401 responses include
  `WWW-Authenticate: Basic realm="rshs"` for browser password dialog support.
- **Shadow file**: Persistent credential store (`username:$hash$...` format).
  CLI credentials (`--user`) can be merged in and optionally written back to disk.
- **TLS**: `TlsListener` implements `axum::serve::Listener` wrapping a `tokio-rustls` acceptor.
  Both HTTP and HTTPS branches call `axum::serve(listener, router)` — fully symmetric.

### Supported Methods

| Method    | Handler            | Module      |
| --------- | ------------------ | ----------- |
| GET/HEAD  | `handle_get_head`  | `http.rs`   |
| PUT       | `handle_put`       | `http.rs`   |
| DELETE    | `handle_delete`    | `http.rs`   |
| OPTIONS   | `handle_options`   | `http.rs`   |
| PROPFIND  | `handle_propfind`  | `webdav.rs` |
| MKCOL     | `handle_mkcol`     | `webdav.rs` |
| COPY      | `handle_copy`      | `webdav.rs` |
| MOVE      | `handle_move`      | `webdav.rs` |
| PROPPATCH | `handle_proppatch` | `webdav.rs` |
| LOCK      | `handle_lock`      | `locks.rs`  |
| UNLOCK    | `handle_unlock`    | `locks.rs`  |

### Body Streaming Pattern

PUT handler uses `StreamReader` + `tokio::io::copy` for zero-copy streaming from HTTP body to file:

```rust
let stream = body.into_data_stream().map_err(std::io::Error::other);
let mut reader = StreamReader::new(stream);
let bytes_written = tokio::io::copy(&mut reader, &mut file).await?;
```

`TryStreamExt::map_err` bridges `axum::Error` → `io::Error` for `StreamReader` compatibility.

### Known Limitations

| Item                      | Status      | Description                                                                                                                                                         |
| ------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Shared lock scope         | TODO (high) | Exclusive write locks only. Shared locks + depth:infinity not yet implemented. Affects litmus tests: `lock_shared` (23), `indirect_refresh` (36)                    |
| Conditional If header     | TODO (high) | `If: (Not <DAV:no-lock>)` and complex conditionals not fully parsed (RFC 4918 §10.4). Affects litmus tests: `cond_put_with_not` (17), `fail_cond_put_unlocked` (22) |
| Collection lock semantics | TODO (high) | Locked collection enforcement for DELETE and owner-token forwarding in COPY not fully handled. Affects litmus tests: `copy` (14), `notowner_modify` (34)            |
| Lock timeout cleanup      | TODO        | No lazy/background cleanup of expired locks                                                                                                                         |
| Dead property persistence | TODO        | In-memory only (`DeadPropertyStore`), lost on restart. xattr/sidecar-file persistence planned                                                                       |
| `getetag` format          | Known       | Uses mtime+size hex hash (`format!("{:x}-{:x}", mtime_secs, size)`). No inode available on macOS via `std::fs`                                                      |
| HTML directory listing    | Known       | Single-line HTML output (no indentation). Adequate for browser rendering                                                                                            |

### Litmus Conformance

| Suite    | Status | Passed | Total | Notes                                                                        |
| -------- | ------ | ------ | ----- | ---------------------------------------------------------------------------- |
| basic    | ✅     | 16     | 16    | 1 warning (delete_fragment)                                                  |
| http     | ✅     | 4      | 4     |                                                                              |
| copymove | ✅     | 13     | 13    | 2 warnings (201 vs 204, RFC 2518 ambiguity)                                  |
| locks    | 🟡     | 24     | 30    | 6 remaining failures documented in Known Limitations; 11 skipped (cascading) |

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
- WebDAV conformance is verified with [litmus](http://www.webdav.org/neon/litmus/); results
  are documented in [`docs/litmus-test-report.md`](docs/litmus-test-report.md)

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

## TLS

TLS/HTTPS is enabled by providing both a certificate and private key file in PEM format:

```sh
rshs --tls-cert cert.pem --tls-key key.pem ./docs
RSHS_TLS_CERT=cert.pem RSHS_TLS_KEY=key.pem rshs ./docs
```

- Default port switches from 8080 to 8443 when TLS is enabled (unless `--port` is explicitly set)
- Certificate SHA-256 fingerprint is logged at startup (colon-separated uppercase hex)
- HTTP/2 negotiation enabled via ALPN (`h2` + `http/1.1`)
- PEM loading failures are logged at `error` level before exiting
- TLS is _not_ auto-detected — both cert and key must be explicitly provided

## Modes

The server always runs in HTTP + WebDAV hybrid mode:

```sh
rshs ./docs                # Serve files in ./docs
rshs                       # Default: serve current directory
RSHS_ROOT_DIR=./docs rshs  # Set root via env var
```

- **Browser**: GET/HEAD → HTML directory listing, file serving
- **WebDAV client**: PROPFIND/PUT/DELETE/MKCOL… → WebDAV protocol

## Health Check

Header-based health check via the `HealthCheck` middleware (sits outermost in the chain, before auth).
Any request with header `x-health-check: true` returns `200 OK` with body `OK`,
regardless of path. Does not require authentication.

```sh
curl -H "x-health-check: true" http://localhost:8080/
# → 200 OK, body: OK
```

- The middleware uses `tower::Layer` pattern for body-type-agnostic interception
- Placed as outermost `.layer()` so it runs before auth and tracing
- Health check requests are logged at `debug` level: `tracing::debug!(%peer, "health check")`

## Logging

Uses the `tracing` ecosystem (structured, span-based) with `tracing-subscriber` as the output backend.

Log level is determined by the following priority (highest first):

1. `-q` / `--quiet` — suppress all logs (`off`)
2. `-vv` / `--verbose --verbose` — trace level
3. `-v` / `--verbose` — debug level
4. `RSHS_LOG` env var — `EnvFilter` string (e.g. `info`, `rshs=debug`, `rshs[status=500]=trace`)
5. Default — `info` level

```sh
rshs -v                                 # debug
rshs -vv                                # trace
rshs -q                                 # silent
rshs                                    # info (or RSHS_LOG if set)
RSHS_LOG="rshs[status=500]=debug" rshs  # only 500 errors at debug
RSHS_LOG="warn,rshs=debug" rshs         # global warn, rshs debug
```

`RSHS_LOG_STYLE` controls ANSI color output (`auto`, `always`, `never`).

# Environment Variables

| Env Var            | Description                                       |
| ------------------ | ------------------------------------------------- |
| `RSHS_ROOT_DIR`    | Root directory (default: `.`)                     |
| `RSHS_HOST`        | Bind address                                      |
| `RSHS_PORT`        | Bind port                                         |
| `RSHS_TLS_CERT`    | TLS certificate file path (PEM format)            |
| `RSHS_TLS_KEY`     | TLS private key file path (PEM format)            |
| `RSHS_USERS`       | Basic Auth credentials                            |
| `RSHS_LOG`         | Logging level (e.g. `info`)                       |
| `RSHS_LOG_STYLE`   | Log output style (e.g. `auto`, `always`, `never`) |
| `RSHS_SHADOW_FILE` | Shadow file path with optional `:rw`/`:ro` suffix |
