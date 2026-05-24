# rshs

[![Build](https://github.com/mogeko/rshs/actions/workflows/build.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/build.yaml)
[![Test](https://github.com/mogeko/rshs/actions/workflows/test.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/test.yaml)
[![Litmus](https://img.shields.io/badge/Litmus-102/102-green)](./docs/litmus-test-report.md)
[![Throughput](https://img.shields.io/badge/Throughput-~2.1K_req/s-blue)](./docs/benchmark-report.md)

A WebDAV server that Just Works — zero config, [litmus 100%](./docs/litmus-test-report.md).

- **Browser + WebDAV hybrid** — HTML directory listings for humans, WebDAV protocol for mounting (Finder, Explorer, davfs).
- **Fast and lightweight** — request latency ~44µs (GET), ~93µs (PUT), 1000-file directory listing in 6ms.
  [Full benchmark report](./docs/benchmark-report.md)
- **Zero config, [one command](#quick-start)** — no config files, daemons, or runtime dependencies beyond Docker.
  Just mount a volume and go.
- **Litmus 102/102** — full [RFC 4918 Class 2](https://datatracker.ietf.org/doc/html/rfc4918) compliance: locks, copy/move,
  conditional `If` headers, dead properties. All five suites pass.
- **TLS + HTTP/2** — built-in HTTPS with automatic HTTP/2 negotiation.
- **Optional Basic Auth** — per-user credentials with persistent shadow files (SHA-512 crypt).

## Installation

```sh
docker pull docker.io/mogeko/rshs:latest
# or
docker pull ghcr.io/mogeko/rshs:latest
```

## Quick Start

```sh
# Serve ./data on port 8080
docker run --rm -p 8080:8080 -v ./data:/mnt/data mogeko/rshs

# With TLS (default port 8443)
docker run --rm -p 8443:8443 \
  -v ./certs:/certs -v ./data:/mnt/data \
  mogeko/rshs --tls-cert /certs/cert.pem --tls-key /certs/key.pem

# With authentication
docker run --rm -p 8080:8080 -v ./data:/mnt/data \
  mogeko/rshs --user admin:secret123
```

Open `http://localhost:8080` in a browser, or mount as WebDAV:

```sh
# Linux (davfs2)
sudo mount -t davfs http://localhost:8080 /mnt/webdav
# macOS (Finder)
Cmd+K → `http://localhost:8080`
# Windows (Explorer)
Map Network Drive → `http://localhost:8080`
```

## Documentation

| Document                             | Description                         |
| ------------------------------------ | ----------------------------------- |
| [Usage Guide][usage-guide]           | Full usage, auth, shadow files, CLI |
| [Docker Compose][docker-compose]     | Docker Compose deployment           |
| [Podman Quadlet][podman-quadlet]     | Podman Quadlet deployment           |
| [Kubernetes][kubernetes]             | K8s deployment, PVC, Ingress        |
| [Benchmark Report][benchmark-report] | Performance benchmarks (52 suites)  |
| [Litmus Test Report][litmus-report]  | WebDAV conformance test results     |

[usage-guide]: ./docs/usage.md
[docker-compose]: ./docs/deploy-docker-compose.md
[podman-quadlet]: ./docs/deploy-podman-quadlet.md
[kubernetes]: ./docs/deploy-k8s.md
[benchmark-report]: ./docs/benchmark-report.md
[litmus-report]: ./docs/litmus-test-report.md

## Environment Variables

| Variable            | Description                                           | Default   |
| ------------------- | ----------------------------------------------------- | --------- |
| `RSHS_ROOT_DIR`     | Root directory to serve                               | `.`       |
| `RSHS_HOST`         | Bind address                                          | `0.0.0.0` |
| `RSHS_PORT`         | Bind port (8080 plain, 8443 with TLS)                 | —         |
| `RSHS_TLS_CERT`     | TLS certificate file path (PEM)                       | —         |
| `RSHS_TLS_KEY`      | TLS private key file path (PEM)                       | —         |
| `RSHS_USERS`        | `user:pass;...` auth pairs                            | —         |
| `RSHS_SHADOW_FILE`  | Shadow file path                                      | —         |
| `RSHS_LOCK_TIMEOUT` | Default WebDAV lock timeout in seconds (default: 300) | `300`     |
| `RSHS_LOG`          | Log filter (e.g. `debug`, `rshs[status=500]=trace`)   | —         |
| `RSHS_LOG_STYLE`    | Log output style                                      | `auto`    |

## License

MIT License. See [LICENSE](./LICENSE) for details.
