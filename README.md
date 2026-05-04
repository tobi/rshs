# rshs

[![Build](https://github.com/mogeko/rshs/actions/workflows/build.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/build.yaml)
[![Test](https://github.com/mogeko/rshs/actions/workflows/test.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/test.yaml)

A lightweight file server with WebDAV support.

- **Browser**: open directories as HTML pages, browse and view files
- **WebDAV client**: mount as a remote drive (Finder, Explorer, `davfs`, etc.)
- **Auth**: optional HTTP Basic Auth for access control

## Installation

```sh
cargo install --git https://github.com/mogeko/rshs
```

Or build from source:

```sh
git clone https://github.com/mogeko/rshs
cd rshs
cargo build --release
```

## Usage

```sh
# Serve the current directory
rshs

# Serve a specific directory
rshs ./docs

# Set root via environment variable
RSHS_ROOT_DIR=/var/www rshs

# Custom host and port
rshs -H 127.0.0.1 -p 3000 ./public
```

### Authentication

```sh
# Single user
rshs --user admin:secret123 ./private

# Multiple users
rshs --user admin:secret --user viewer:public ./private

# Via environment variable
RSHS_USERS="admin:secret;viewer:public" rshs ./private
```

Credentials format: `username:password`, separated by `;` for multiple users.
CLI values take precedence over environment variables.

### Access

| Client           | How to access                                                |
| ---------------- | ------------------------------------------------------------ |
| Browser          | Open `http://localhost:8080`                                 |
| macOS Finder     | `Cmd+K` → `http://localhost:8080`                            |
| Windows Explorer | Map network drive → `http://localhost:8080`                  |
| Linux davfs2     | `mount.davfs http://localhost:8080 /mnt`                     |
| curl             | `curl http://localhost:8080` (GET) / `curl -X PROPFIND http://localhost:8080` |

## CLI Reference

```
Usage: rshs [OPTIONS] [ROOT_DIR]

Arguments:
  [ROOT_DIR]  Root directory to serve [env: RSHS_ROOT_DIR=] [default: .]

Options:
  -H, --host <HOST>       Host address to bind to [env: RSHS_HOST=] [default: 0.0.0.0]
  -p, --port <PORT>       Port to bind to [env: RSHS_PORT=] [default: 8080]
  -u, --user <USER:PASS>  Basic Auth credentials in format username:password (can be repeated) [env: RSHS_USERS=]
  -h, --help
```

## Environment Variables

| Variable        | Description           | Default   |
| --------------- | --------------------- | --------- |
| `RSHS_ROOT_DIR` | Root directory        | `.`       |
| `RSHS_HOST`     | Bind address          | `0.0.0.0` |
| `RSHS_PORT`     | Bind port             | `8080`    |
| `RSHS_USERS`    | `user:pass;...` pairs | —         |

## License

MIT License. See [LICENSE](LICENSE) for details.
