# rshs

[![Build](https://github.com/mogeko/rshs/actions/workflows/build.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/build.yaml)
[![Test](https://github.com/mogeko/rshs/actions/workflows/test.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/test.yaml)

A lightweight file server with WebDAV support.

- **Browser**: open directories as HTML pages, browse and view files
- **WebDAV client**: mount as a remote drive (Finder, Explorer, `davfs`, etc.)
- **WebDAV locks**: shared + exclusive locks, depth:infinity, conditional `If` header (RFC 4918 §10.4)
- **Auth**: optional HTTP Basic Auth for access control
- **TLS/HTTPS**: built-in support for secure connections with custom certs
- **HTTP/2**: automatic HTTP/2 support when using TLS (ALPN negotiation)
- **WebDAV conformance**: 35/37 on litmus (94.6%) — basic/http/copymove 100%; 2 litmus 0.14 deviations ([report][litmus-report])

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

| Document                            | Description                         |
| ----------------------------------- | ----------------------------------- |
| [Usage Guide][usage-guide]          | Full usage, auth, shadow files, CLI |
| [Docker Compose][docker-compose]    | Docker Compose deployment           |
| [Podman Quadlet][podman-quadlet]    | Podman Quadlet deployment           |
| [Kubernetes][kubernetes]            | K8s deployment, PVC, Ingress        |
| [Litmus Test Report][litmus-report] | WebDAV conformance test results     |

[usage-guide]: ./docs/usage.md
[docker-compose]: ./docs/deploy-docker-compose.md
[podman-quadlet]: ./docs/deploy-podman-quadlet.md
[kubernetes]: ./docs/deploy-k8s.md
[litmus-report]: ./docs/litmus-test-report.md

## Environment Variables

| Variable           | Description                                         | Default   |
| ------------------ | --------------------------------------------------- | --------- |
| `RSHS_ROOT_DIR`    | Root directory to serve                             | `.`       |
| `RSHS_HOST`        | Bind address                                        | `0.0.0.0` |
| `RSHS_PORT`        | Bind port (8080 plain, 8443 with TLS)               | —         |
| `RSHS_TLS_CERT`    | TLS certificate file path (PEM)                     | —         |
| `RSHS_TLS_KEY`     | TLS private key file path (PEM)                     | —         |
| `RSHS_USERS`       | `user:pass;...` auth pairs                          | —         |
| `RSHS_SHADOW_FILE` | Shadow file path                                    | —         |
| `RSHS_LOG`         | Log filter (e.g. `debug`, `rshs[status=500]=trace`) | —         |
| `RSHS_LOG_STYLE`   | Log output style                                    | `auto`    |

## License

MIT License. See [LICENSE](./LICENSE) for details.
