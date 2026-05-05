# rshs

[![Build](https://github.com/mogeko/rshs/actions/workflows/build.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/build.yaml)
[![Test](https://github.com/mogeko/rshs/actions/workflows/test.yaml/badge.svg)](https://github.com/mogeko/rshs/actions/workflows/test.yaml)

A lightweight file server with WebDAV support.

- **Browser**: open directories as HTML pages, browse and view files
- **WebDAV client**: mount as a remote drive (Finder, Explorer, `davfs`, etc.)
- **Auth**: optional HTTP Basic Auth for access control

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

| Document                                  | Description                         |
| ----------------------------------------- | ----------------------------------- |
| [Usage Guide][usage-guide]                | Full usage, auth, shadow files, CLI |
| [Docker & Docker Compose][docker-compose] | Docker + docker-compose deployment  |
| [Kubernetes][kubernetes]                  | K8s deployment, PVC, Ingress        |

[usage-guide]:./docs/usage.md
[docker-compose]:./docs/deploy-docker-compose.md
[kubernetes]:./docs/deploy-k8s.md

## Environment Variables

| Variable           | Description                | Default   |
| ------------------ | -------------------------- | --------- |
| `RSHS_ROOT_DIR`    | Root directory to serve    | `.`       |
| `RSHS_HOST`        | Bind address               | `0.0.0.0` |
| `RSHS_PORT`        | Bind port                  | `8080`    |
| `RSHS_USERS`       | `user:pass;...` auth pairs | —         |
| `RSHS_SHADOW_FILE` | Shadow file path           | —         |
| `RSHS_LOG`         | Log level (e.g. `debug`)   | —         |
| `RSHS_LOG_STYLE`   | Log output style           | `auto`    |

## License

MIT License. See [LICENSE](./LICENSE) for details.


