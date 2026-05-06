# Docker Compose

## Basic

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

## With Authentication

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

## With Persistent Shadow File

Generate the shadow file with SHA-512 crypt hashes, then mount it as a
[Docker secret](https://docs.docker.com/compose/how-tos/use-secrets/):

```sh
# Generate a hashed password
openssl passwd -6 "secret123"
# → $6$xxxxxxxx$yyyyyyyyyyyyyyyyyyyyyyyyyyyy...

# Write the shadow file (one user per line: username:hash)
echo 'admin:$6$xxxxxxxx$yyyyyyyyyyyyyyyyyyyyyyyyyyyy...' > ./rshs/shadow
echo 'viewer:$6$aaaaaaaa$bbbbbbbbbbbbbbbbbbbbbb...' >> ./rshs/shadow
```

```yaml
# docker-compose.yml
services:
  rshs:
    image: docker.io/mogeko/rshs:latest
    ports:
      - "8080:8080"
    volumes:
      - ./data:/mnt/data
    environment:
      RSHS_SHADOW_FILE: /run/secrets/rshs-shadow:ro
    secrets:
      - rshs-shadow
    restart: unless-stopped

secrets:
  rshs-shadow:
    file: ./rshs/shadow
```

Docker Compose mounts secrets into `/run/secrets/<name>` in the container.
`RSHS_SHADOW_FILE` points to the secret mount with `:ro` (read-only).
To update credentials, regenerate the shadow file and restart the service.

## Health Check

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
