# Docker & Docker Compose

## Docker Run

```sh
# Basic: serve ./data on port 8080
docker run --rm -p 8080:8080 -v ./data:/mnt/data mogeko/rshs

# With authentication
docker run --rm -p 8080:8080 \
  -v ./data:/mnt/data \
  mogeko/rshs --user admin:secret123

# Custom port and host
docker run --rm -p 3000:3000 \
  -v ./data:/mnt/data \
  mogeko/rshs --host 0.0.0.0 --port 3000
```

### Persistent Shadow File

Mount a directory for the shadow file so credentials survive container restarts:

```sh
docker run --rm -p 8080:8080 \
  -v ./data:/mnt/data \
  -v ./rshs/shadow:/etc/rshs/shadow \
  -e RSHS_USERS="admin:secret123" \
  mogeko/rshs -W
```

This writes the hashed credentials to `./rshs/shadow` on the host. On subsequent
runs, existing credentials are loaded from the shadow file automatically (the image
sets `RSHS_SHADOW_FILE=rw:/etc/rshs/shadow` by default).

## Docker Compose

### Basic

```yaml
# docker-compose.yml
services:
  rshs:
    image: mogeko/rshs:latest
    ports:
      - "8080:8080"
    volumes:
      - ./rshs/data:/mnt/data
    restart: unless-stopped
```

### With Authentication

```yaml
# docker-compose.yml
services:
  rshs:
    image: mogeko/rshs:latest
    ports:
      - "8080:8080"
    volumes:
      - ./data:/mnt/data
    environment:
      RSHS_USERS: "admin:secret123;viewer:public"
    restart: unless-stopped
```

### With Persistent Shadow File

```yaml
# docker-compose.yml
services:
  rshs:
    image: mogeko/rshs:latest
    ports:
      - "8080:8080"
    volumes:
      - ./rshs/data:/mnt/data
      - ./rshs/shadow:/etc/rshs/shadow
    environment:
      RSHS_USERS: "admin:secret123;viewer:public"
    command: ["-W"]
    restart: unless-stopped
```

On first start, the CLI credentials are hashed and written to `./rshs/shadow`.
On subsequent starts, remove the `command: ["-W"]` line (or keep it — `-W` is
harmless if credentials haven't changed).

### Health Check

```yaml
# docker-compose.yml
services:
  rshs:
    image: mogeko/rshs:latest
    ports:
      - "8080:8080"
    volumes:
      - ./data:/mnt/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/"]
      interval: 30s
      timeout: 10s
      retries: 3
    restart: unless-stopped
```
