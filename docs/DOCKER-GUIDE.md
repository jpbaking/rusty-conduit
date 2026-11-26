# Docker Guide

This guide covers running `rusty-conduit` in a container with `docker run` and Docker Compose: the published images, configuration files, logs, HTTPS MITM, resource limits, health checks, and hardening.

- Back to [README](../README.md) · [Basic User Guide](USER-GUIDE-BASIC.md) · [Advanced User Guide](USER-GUIDE-ADVANCED.md) · [Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md)

The examples were tested with Docker Engine 29.9 and Docker Compose 5.7 on Docker Desktop (Linux arm64). The file-ownership notes for Linux hosts were checked with files owned by another UID.

---

## 1. Images

| | |
| :--- | :--- |
| **Image** | `ghcr.io/jpbaking/rusty-conduit` |
| **Tags** | `X.Y.Z`, `X.Y`, `X`, and `latest` (the newest stable release). A pre-release gets only its full version tag. |
| **Platforms** | `linux/amd64`, `linux/arm64` |
| **Base** | Ubuntu 26.04 LTS with `ca-certificates` |
| **User** | `rusty-conduit`, UID and GID `10001`, set as a numeric `USER 10001:10001` so orchestrators can verify it is not root (Kubernetes `runAsNonRoot`). Nothing in the image runs as root. |
| **Entrypoint** | `/usr/local/bin/rusty-conduit`. Anything after the image name is passed to it as arguments. |
| **Ports** | `3128` (HTTP/HTTPS) and `1080` (SOCKS4/4a/5) |
| **Environment** | `RUSTY_CONDUIT_BIND=0.0.0.0`, so the listeners accept connections forwarded by Docker, and `MALLOC_ARENA_MAX=2`, which keeps memory low on hosts with many CPUs |

Pin a major tag such as `:2` so you get fixes and features within that major version, but not breaking changes. To build the image yourself from a clone of the repository:

```sh
docker build -t rusty-conduit .
```

---

## 2. Run with `docker run`

```sh
docker run -d --name rusty-conduit --restart unless-stopped \
  -p 127.0.0.1:3128:3128 \
  -p 127.0.0.1:1080:1080 \
  ghcr.io/jpbaking/rusty-conduit:2
```

- **Publish on `127.0.0.1`.** Inside the container the proxy listens on `0.0.0.0`, so the `SECURITY NOTICE` at startup is expected. What decides who can reach it is the host side of `-p`: `-p 3128:3128` without an address exposes the proxy to your network, and there is **no proxy authentication**.
- **Flags and environment variables** work as usual: put flags after the image name, or pass `-e RUSTY_CONDUIT_*` variables.
- **Memory:** the image sets `MALLOC_ARENA_MAX=2`, which keeps memory low on hosts with many CPUs. Leave it in place unless you measure otherwise; see [Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md#memory-on-many-core-linux-malloc_arena_max).
- **Logs:** `docker logs -f rusty-conduit`.
- **Stop:** `docker stop rusty-conduit` sends `SIGTERM`. Active connections drain for up to 5 seconds, well inside Docker's default 10-second grace period.

Serve HTTP, HTTPS, and SOCKS on one port:

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:8888:8888 \
  ghcr.io/jpbaking/rusty-conduit:2 --http-port 8888 --socks-port 8888
```


### File ownership on Linux hosts

Mounted files keep their host owner, and the image never runs as root, so it cannot fix ownership or modes at startup. This matters when you mount a CA key, a passphrase file, or a log directory:

| Host | Do this |
| :--- | :--- |
| **Linux (recommended)** | Run the container as your own UID: `--user "$(id -u):$(id -g)"` (Compose: `user: "1000:1000"`, with your numbers). Your files stay yours, at their usual modes, with no `sudo`. The process is still not root. A named volume at `/var/log/rusty-conduit` belongs to UID `10001`, so write logs to a host directory you own instead. |
| **Linux, image user** | Keep UID `10001` and hand it the files: `sudo chown 10001 <file or directory>`, keeping secret files at mode `600`. |
| **Docker Desktop** (macOS, Windows) | Nothing: Docker Desktop maps file ownership to the container user. |

---

## 3. Configuration file

Mount a **directory** read-only and point `--config` at the file inside it:

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 \
  -v "$PWD/conf:/etc/rusty-conduit:ro" \
  ghcr.io/jpbaking/rusty-conduit:2 --config /etc/rusty-conduit/rusty-conduit.toml
```

Check it before starting:

```sh
docker run --rm -v "$PWD/conf:/etc/rusty-conduit:ro" \
  ghcr.io/jpbaking/rusty-conduit:2 validate --config /etc/rusty-conduit/rusty-conduit.toml
# config OK: /etc/rusty-conduit/rusty-conduit.toml
```

`validate` checks the settings, not the files they name: an unreadable CA key or log directory still passes, and fails only when the proxy starts. Environment variables override the file as usual, for example `-e RUSTY_CONDUIT_LOG_LEVEL=DEBUG`. See the [Advanced User Guide](USER-GUIDE-ADVANCED.md) for every key.

---

## 4. Logs

Diagnostics go to `stderr`, which `docker logs` shows. For HTTP access and SOCKS connection records, choose one of these:

**On `stdout`.** At `--log-level DEBUG`, records are also written to `stdout`, so Docker's log driver collects them. Diagnostics stay on `stderr`, so you can split the two:

```sh
docker logs rusty-conduit 2>/dev/null   # access and SOCKS records only
```

**In files.** Mount a volume at the image's log directory and point the log flags at it:

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 \
  -v rusty-conduit-logs:/var/log/rusty-conduit \
  ghcr.io/jpbaking/rusty-conduit:2 \
  --http-access-log /var/log/rusty-conduit/access.log \
  --socks-log /var/log/rusty-conduit/socks.log

docker exec rusty-conduit tail -f /var/log/rusty-conduit/access.log
```

- The image's `/var/log/rusty-conduit` belongs to the container user (UID `10001`), so a new named volume mounted there is writable as is. To keep the files in a host directory instead, mount it there (`-v "$PWD/logs:/var/log/rusty-conduit"`) and make it writable by the container's UID ([File ownership](#file-ownership-on-linux-hosts)).
- Records reach the files as soon as the proxy has no more queued, so `tail -f` follows them live; `docker stop` flushes the rest at shutdown.

---

## 5. HTTPS MITM

MITM needs a CA you create yourself; see the [MITM CA Guide](MITM-CA-GUIDE.md). Mount the CA files read-only, and make sure the container's UID can read the key ([File ownership](#file-ownership-on-linux-hosts)):

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 \
  -v "$PWD/ca:/etc/rusty-conduit/ca:ro" \
  ghcr.io/jpbaking/rusty-conduit:2 \
  --mitm-ca-cert /etc/rusty-conduit/ca/ca.pem \
  --mitm-ca-key /etc/rusty-conduit/ca/ca.key
```

- An unreadable key stops the proxy at startup with `FATAL MITM setup failed: failed to read MITM CA file ...: Permission denied`.
- For an encrypted key or PKCS#12 bundle, pass the passphrase as a file (`--mitm-ca-passphrase-file`) mounted the same way, or as a Compose secret ([section 9.2](#92-config-file-logs-mitm-limits-and-a-health-check)). `docker inspect` shows command-line arguments and environment variables to anyone with access to the Docker daemon.
- Clients must trust the CA. Test one request with `curl --cacert ca/ca.pem -x http://127.0.0.1:3128 https://example.com/`.

---

## 6. Resource limits

| Limit | Inside the container | What to do |
| :--- | :--- | :--- |
| Open files (`ulimit -n`) | Usually high: `1048576` on Docker Desktop. The proxy raises its soft limit toward the hard limit at startup. | If startup logs `NOTICE: open-file limit is ...`, add `--ulimit nofile=65536`, or lower `--max-connections`. It needs `2 × max_connections + 64`. |
| Listen backlog (`somaxconn`) | The container has its own network namespace; the default is `4096` on Linux 5.4 and later. | On older kernels, add `--sysctl net.core.somaxconn=4096`. With `--network host`, the host's value applies and `--sysctl` is not allowed. |
| Memory | The image sets `MALLOC_ARENA_MAX=2`, so memory stays low on any CPU count. | Add `--memory` as a ceiling if you want one. |
| CPUs | One worker thread per CPU the container may use. | `--cpus=2` also reduces idle memory. |

[Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md) has the measured figures and sizing rules.

---

## 7. Health checks

The image has no `curl` or `nc`, but it has `bash`, which can open a TCP connection to the listener:

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 \
  --health-cmd 'bash -c "exec 3<>/dev/tcp/127.0.0.1/3128"' \
  --health-interval 30s --health-timeout 3s --health-retries 3 \
  ghcr.io/jpbaking/rusty-conduit:2
```

The check passes while the HTTP listener accepts connections. If you disable the HTTP port, check the SOCKS port instead.

---

## 8. Hardening

The proxy writes nothing except the log files you configure, and needs no Linux capabilities. These options were tested with every example in this guide:

```sh
docker run -d --name rusty-conduit \
  -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 \
  --read-only --cap-drop ALL --security-opt no-new-privileges:true \
  ghcr.io/jpbaking/rusty-conduit:2
```

Keep the published ports on `127.0.0.1`, or behind a firewall, until proxy authentication exists.

---

## 9. Docker Compose

Save an example as `compose.yaml`, then:

```sh
docker compose up -d
docker compose logs -f
docker compose down
```

### 9.1 Minimal

```yaml
services:
  rusty-conduit:
    image: ghcr.io/jpbaking/rusty-conduit:2
    restart: unless-stopped
    ports:
      - "127.0.0.1:3128:3128"
      - "127.0.0.1:1080:1080"
```

### 9.2 Config file, logs, MITM, limits, and a health check

The layout next to `compose.yaml`:

```text
conf/rusty-conduit.toml
logs/                # Linux hosts: see File ownership in section 2
secrets/ca.pem
secrets/ca.key       # Linux hosts: see File ownership in section 2
```

`conf/rusty-conduit.toml` reads the CA from the paths where Compose mounts secrets:

```toml
socks_block_web = true

[mitm]
ca_cert = "/run/secrets/ca_cert"
ca_key = "/run/secrets/ca_key"

[logging]
format = "json"
http_access_log = "/var/log/rusty-conduit/access.log"
socks_log = "/var/log/rusty-conduit/socks.log"
```

`compose.yaml`:

```yaml
services:
  rusty-conduit:
    image: ghcr.io/jpbaking/rusty-conduit:2
    restart: unless-stopped
    command: ["--config", "/etc/rusty-conduit/rusty-conduit.toml"]
    ports:
      - "127.0.0.1:3128:3128"
      - "127.0.0.1:1080:1080"
    volumes:
      - ./conf:/etc/rusty-conduit:ro
      - ./logs:/var/log/rusty-conduit
    secrets:
      - ca_cert
      - ca_key
    read_only: true
    cap_drop: [ALL]
    security_opt: ["no-new-privileges:true"]
    ulimits:
      nofile: 65536
    sysctls:
      net.core.somaxconn: 4096
    healthcheck:
      test: ["CMD", "bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/3128"]
      interval: 30s
      timeout: 3s
      retries: 3
      start_period: 5s

secrets:
  ca_cert:
    file: ./secrets/ca.pem
  ca_key:
    file: ./secrets/ca.key
```

Check the configuration with the same mounts before starting:

```sh
docker compose run --rm --no-deps rusty-conduit validate --config /etc/rusty-conduit/rusty-conduit.toml
```

### 9.3 A proxy for other containers

Services on the same Compose network reach the proxy by its service name. This proxy publishes no ports, so nothing outside the network can use it:

```yaml
services:
  proxy:
    image: ghcr.io/jpbaking/rusty-conduit:2
    restart: unless-stopped

  app:
    image: curlimages/curl:latest
    command: ["-sS", "-o", "/dev/null", "-w", "%{http_code}\n", "https://example.com/"]
    environment:
      HTTP_PROXY: http://proxy:3128
      HTTPS_PROXY: http://proxy:3128
      NO_PROXY: localhost,127.0.0.1
    depends_on:
      - proxy
```

Tools differ in which names they read: `curl`, for example, ignores `HTTP_PROXY` and reads only the lowercase `http_proxy` for `http://` URLs. Set both forms if a client ignores the proxy. For SOCKS, use `ALL_PROXY: socks5h://proxy:1080`.

---

## 10. Troubleshooting

| Symptom | Cause | Fix |
| :--- | :--- | :--- |
| `FATAL failed to open access log: ... Permission denied` | The container's UID cannot create files in the log directory | Use a named volume, or a host directory the container's UID can write ([File ownership](#file-ownership-on-linux-hosts)) |
| `FATAL MITM setup failed: failed to read MITM CA file ...: Permission denied` | The key file is not readable by the container's UID | Run as your own UID, or `sudo chown 10001` the key ([File ownership](#file-ownership-on-linux-hosts)) |
| Clients on other machines cannot connect | The ports are published on `127.0.0.1` | Publish on a LAN address or `0.0.0.0` only behind a firewall: there is no proxy authentication |
| `SECURITY NOTICE: binding to non-loopback address 0.0.0.0` at startup | Expected: the proxy listens on all interfaces inside the container | Control exposure with the host side of `-p` |
| Memory is much higher than the sizing guide says | `MALLOC_ARENA_MAX` was overridden or unset, so glibc allocates a memory arena per worker thread | Keep the image default, `MALLOC_ARENA_MAX=2` ([section 6](#6-resource-limits)) |
| `docker: Error response from daemon: ... Bind for 127.0.0.1:3128 failed` | Another process on the host uses the port | Change the host side only, for example `-p 127.0.0.1:8080:3128` |
