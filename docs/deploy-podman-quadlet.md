# Podman Quadlet

Quadlet is Podman's native container-to-systemd integration. Place a `.container` file
in `~/.config/containers/systemd/`, and Podman auto-generates a systemd user service.
No daemon required — the container runs directly via `systemctl --user`.

For system-wide deployments, use `/etc/containers/systemd/` instead.

> [!NOTE]
> Requires Podman 4.4+ with Quadlet support.

## Basic

Create `~/.config/containers/systemd/rshs.container`:

```ini
[Container]
Image=docker.io/mogeko/rshs:latest
PublishPort=8080:8080
Volume=%h/data:/mnt/data

[Service]
Restart=always

[Install]
WantedBy=default.target
```

`%h` expands to the user's home directory. Replace with an absolute path if needed.

## With Authentication

```ini
[Container]
Image=docker.io/mogeko/rshs:latest
PublishPort=8080:8080
Volume=%h/data:/mnt/data
Environment=RSHS_USERS=admin:secret123;viewer:public

[Service]
Restart=always

[Install]
WantedBy=default.target
```

Credentials format: `username:password`, separated by `;` for multiple users.

## With Persistent Shadow File

Use [Podman secrets](https://docs.podman.io/en/latest/markdown/podman-secret-create.1.html)
to store the shadow file securely — no bind-mount or plaintext exposure.

First, generate the shadow file with SHA-512 crypt hashes:

```sh
# Generate a hashed password
openssl passwd -6 "secret123"
# → $6$xxxxxxxx$yyyyyyyyyyyyyyyyyyyyyyyyyyyy...

# Write the shadow file (one user per line: username:hash)
echo 'admin:$6$xxxxxxxx$yyyyyyyyyyyyyyyyyyyyyyyyyyyy...' > ~/rshs-shadow

# Create a Podman secret
podman secret create rshs-shadow ~/rshs-shadow

# Remove the plaintext local copy
rm ~/rshs-shadow
```

Then reference it in the Quadlet:

```ini
[Container]
Image=docker.io/mogeko/rshs:latest
PublishPort=8080:8080
Volume=%h/data:/mnt/data
Secret=rshs-shadow,type=mount,target=/etc/rshs/shadow,mode=0400
Environment=RSHS_SHADOW_FILE=/etc/rshs/shadow:ro

[Service]
Restart=always

[Install]
WantedBy=default.target
```

- `type=mount` mounts the secret as a regular file (default is `env` which exposes it as an env var)
- `mode=0400` restricts the file to read-only for the owner
- `:ro` in `RSHS_SHADOW_FILE` ensures rshs won't attempt to write back

To update credentials later, recreate the secret and restart:

```sh
podman secret rm rshs-shadow
echo 'admin:$6$newhash...' > ~/rshs-shadow
podman secret create rshs-shadow ~/rshs-shadow
systemctl --user restart rshs
```

## Health Check

```ini
[Container]
Image=docker.io/mogeko/rshs:latest
PublishPort=8080:8080
Volume=%h/data:/mnt/data
HealthCmd=curl -f -H "x-health-check: true" http://localhost:8080/
HealthInterval=30s
HealthRetries=3
HealthTimeout=10s

[Service]
Restart=always

[Install]
WantedBy=default.target
```

The `x-health-check: true` header triggers rshs's health check middleware,
which returns `200 OK` without touching the file system or requiring auth.

## Deploy

```sh
# Create the systemd directory if it doesn't exist
mkdir -p ~/.config/containers/systemd

# Place your rshs.container file there, then:
systemctl --user daemon-reload
systemctl --user start rshs

# Enable auto-start at login
systemctl --user enable rshs

# Check status
systemctl --user status rshs
```

> [!TIP]
> Enable lingering if you want the container to start at boot (before login):
>
> ```sh
> sudo loginctl enable-linger $USER
> ```
