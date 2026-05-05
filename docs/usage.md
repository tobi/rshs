# Usage

## Docker Quick Start

```sh
# Serve the current directory
docker run --rm -p 8080:8080 -v .:/mnt/data mogeko/rshs

# Serve a specific directory
docker run --rm -p 8080:8080 -v /path/to/serve:/mnt/data mogeko/rshs

# Custom host and port
docker run --rm -p 3000:3000 -v .:/mnt/data mogeko/rshs --port 3000
```

## Authentication

Basic HTTP Authentication (RFC 7617) is supported via `--user` / `-u` and `RSHS_USERS` env var.

```sh
# Single user
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
If no users are configured, the server runs without authentication.

## Shadow Files

Shadow files provide persistent credential storage with SHA-512 crypt hashing:

```sh
# Load credentials from a shadow file
rshs -S ./shadow ./docs

# Read-only shadow file (no writes allowed)
rshs -S /etc/rshs/shadow:ro ./docs

# Merge CLI credentials and write to shadow file
rshs -S ./shadow:rw -W --user admin:newpass ./docs

# Using environment variable
RSHS_SHADOW_FILE=./shadow:ro rshs ./docs
```

- Path suffix `:rw` (default) allows writes, `:ro` restricts to read-only
- `-W` / `--shadow-write` writes CLI credentials into the shadow file after merging
- Shadow files store passwords as SHA-512 crypt hashes (`$6$...`), compatible with Unix shadow file format

> [!TIP]
> Since the RSHS shadow file is compatible with Unix shadow. You can mount the Linux shadow file as read-only into the container. This allows you to use existing system credentials for authentication.
>
> ```sh
> docker run --rm -p 8080:8080 \
>   -e RSHS_SHADOW_FILE=/etc/rshs/shadow:ro \
>   -v /etc/shadow:/etc/rshs/shadow:ro \
>   mogeko/rshs

## Access

| Client           | How to access                                                |
| ---------------- | ------------------------------------------------------------ |
| Browser          | Open `http://localhost:8080`                                 |
| macOS Finder     | `Cmd+K` → `http://localhost:8080`                            |
| Windows Explorer | Map network drive → `http://localhost:8080`                  |
| Linux davfs2     | `mount.davfs http://localhost:8080 /mnt`                     |
| curl             | `curl http://localhost:8080` (GET) / `curl -X PROPFIND http://localhost:8080` |

## Logging

Log level is determined by the following priority (highest first):

1. `-q` / `--quiet` — suppress all logs (`off`)
2. `-vv` / `--verbose --verbose` — trace level
3. `-v` / `--verbose` — debug level
4. `RSHS_LOG` env var — arbitrary filter string (e.g. `info`, `rshs=debug`)
5. Default — `info` level

```sh
# Default: info level
docker run --rm -p 8080:8080 mogeko/rshs

# Debug level
docker run --rm -p 8080:8080 mogeko/rshs -v

# Trace level (most verbose)
docker run --rm -p 8080:8080 mogeko/rshs -vv

# Suppress all logs
docker run --rm -p 8080:8080 mogeko/rshs -q

# Using environment variable for log level
docker run --rm -p 8080:8080 -e RSHS_LOG="debug" mogeko/rshs
```

`RSHS_LOG_STYLE` controls log output style (`auto`, `always`, `never`).

## CLI Reference

```plaintext
Simple HTTP/WebDAV Server

Usage: rshs [OPTIONS] [ROOT_DIR]

Arguments:
  [ROOT_DIR]  Root directory to serve [env: RSHS_ROOT_DIR=] [default: .]

Options:
  -H, --host <HOST>                  Host address to bind to [env: RSHS_HOST=] [default: 0.0.0.0]
  -p, --port <PORT>                  Port to bind to [env: RSHS_PORT=] [default: 8080]
  -v, --verbose...                   Increase log verbosity (-v = debug, -vv = trace)
  -q, --quiet                        Suppress all log output
  -u, --user <USER:PASS>             Basic Auth credentials in format username:password (can be repeated) [env: RSHS_USERS]
  -S, --shadow-file <PATH[:rw|:ro]>  Path to shadow file for persistent auth (PATH[:rw|:ro], default :rw) [env: RSHS_SHADOW_FILE=]
  -W, --shadow-write                 Write CLI credentials into the shadow file (requires --shadow-file :rw)
  -h, --help                         Print help
  -V, --version                      Print version

Logging environment variables:
  RSHS_LOG          Logging level filter (e.g. info, rshs=debug)
                    Only used when no -v/-q flags are given
  RSHS_LOG_STYLE    Log output style: auto, always, never
```
