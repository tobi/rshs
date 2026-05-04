# rshs

[![Build](https://github.com/mogeko/rshs/actions/workflows/build.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/build.yaml)
[![Test](https://github.com/mogeko/rshs/actions/workflows/test.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/test.yaml)

A lightweight file server with WebDAV support.

- **Browser**: open directories as HTML pages, browse and view files
- **WebDAV client**: mount as a remote drive (Finder, Explorer, `davfs`, etc.)
- **Auth**: optional HTTP Basic Auth for access control

## Installation

```sh
# Docker Hub
docker pull docker.io/mogeko/rshs:latest

# GitHub Container Registry
docker pull ghcr.io/mogeko/rshs:latest
```

## Usage

```sh
# Serve the current directory
docker run --rm -p 8080:8080 -v .:/mnt/data mogeko/rshs

# Serve a specific directory
docker run --rm -p 8080:8080 -v /path/to/serve:/mnt/data mogeko/rshs

# Custom host and port
docker run --rm -p 3000:3000 -v .:/mnt/data mogeko/rshs --port 3000
```

### Authentication

```sh
# With authentication
docker run --rm -p 8080:8080 -v ./private:/mnt/data mogeko/rshs --user admin:secret123

# Multiple users
docker run --rm -p 8080:8080 -v ./private:/mnt/data \
  mogeko/rshs --user admin:secret --user viewer:public

# Using environment variables
docker run --rm -p 3000:3000 \
  -e RSHS_USERS="admin:secret;viewer:public" \
  -v .:/mnt/data \
  mogeko/rshs
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
