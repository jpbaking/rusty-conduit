# Basic User Guide

This guide gets you up and running with `rusty-conduit` for everyday local and homelab proxying.

- Looking for TOML configs, MITM inspection, web-blocking over SOCKS, or JSON log pipelines? See the **[Advanced User Guide](USER-GUIDE-ADVANCED.md)**.
- Curious about RAM, CPU, or `ulimit -n` sizing? See **[Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md)**.
- Running in Docker or Compose? See the **[Docker Guide](DOCKER-GUIDE.md)**.

---

## 1. Install

### Option A: Download a prebuilt binary (recommended)

Grab the archive for your OS and CPU architecture from the [GitHub Releases](https://github.com/jpbaking/rusty-conduit/releases) page:

- **Linux:** `rusty-conduit-<version>-linux-amd64.tar.gz` or `linux-arm64.tar.gz`
- **macOS:** `rusty-conduit-<version>-macos-arm64.tar.gz` (Apple Silicon) or `macos-amd64.tar.gz` (Intel)
- **Windows:** `rusty-conduit-<version>-windows-amd64.zip` or `windows-arm64.zip`

Extract it and place `rusty-conduit` (or `rusty-conduit.exe`) somewhere on your `PATH`.

### Option B: Run with Docker

Multi-arch images (`linux/amd64` and `linux/arm64`, based on Ubuntu 26.04 LTS) are published to GitHub Container Registry on each release:

```sh
# Publish to host loopback only (recommended — v1 has no proxy authentication)
docker run --rm \
  -p 127.0.0.1:3128:3128 \
  -p 127.0.0.1:1080:1080 \
  ghcr.io/jpbaking/rusty-conduit:latest
```

Inside the container, `RUSTY_CONDUIT_BIND` defaults to `0.0.0.0` and the process runs as a non-root user (`rusty-conduit`, UID `10001`). Any CLI flags or `RUSTY_CONDUIT_*` environment variables work as usual:

```sh
docker run --rm \
  -p 127.0.0.1:8888:8888 \
  ghcr.io/jpbaking/rusty-conduit:latest \
  --http-port 8888 --socks-port 8888 --log-level DEBUG
```

The **[Docker Guide](DOCKER-GUIDE.md)** covers config files, logs, MITM, resource limits, and Compose examples.

### Option C: Build from source

Requires a stable Rust toolchain (<https://rustup.rs>):

```sh
git clone https://github.com/jpbaking/rusty-conduit.git
cd rusty-conduit
cargo install --path .
```

---

## 2. Start the Proxy

Run with no arguments to start with safe local defaults:

```sh
rusty-conduit
```

By default, `rusty-conduit` binds only to loopback (`127.0.0.1`) with:

- **HTTP/HTTPS proxy:** `127.0.0.1:3128`
- **SOCKS4 / SOCKS4a / SOCKS5 proxy:** `127.0.0.1:1080`
- **MITM mode:** `OFF` (HTTPS `CONNECT` and SOCKS act as pure encrypted tunnels)
- **Log level:** `INFO`

Press **`Ctrl+C`** (or send `SIGTERM` on Linux/macOS) to shut down cleanly. Active connections drain for up to 5 seconds and log files flush before exit.

---

## 3. Connect Your Clients

### Test with `curl`

```sh
# Plain HTTP via HTTP proxy
curl -x http://127.0.0.1:3128 http://example.com/

# HTTPS via HTTP CONNECT tunnel (opaque, end-to-end encrypted)
curl -x http://127.0.0.1:3128 https://example.com/

# HTTPS via SOCKS5 (proxy resolves DNS)
curl --socks5-hostname 127.0.0.1:1080 https://example.com/

# HTTP via SOCKS4a (proxy resolves DNS)
curl --socks4a 127.0.0.1:1080 http://example.com/
```

### Standard CLI environment variables

Most CLI tools (`curl`, `git`, `python`, `npm`, `cargo`, etc.) respect:

```sh
# Linux / macOS
export HTTP_PROXY="http://127.0.0.1:3128"
export HTTPS_PROXY="http://127.0.0.1:3128"
# Or route everything through SOCKS5:
export ALL_PROXY="socks5h://127.0.0.1:1080"
```

```powershell
# Windows PowerShell
$env:HTTP_PROXY  = "http://127.0.0.1:3128"
$env:HTTPS_PROXY = "http://127.0.0.1:3128"
```

### Browsers (Firefox, Chrome, Edge, Safari)

- **HTTP / HTTPS Proxy:** host `127.0.0.1`, port `3128`
- **SOCKS5 Host:** host `127.0.0.1`, port `1080` (enable *"Proxy DNS when using SOCKS v5"* if available)

---

## 4. Common Everyday Options

### Change ports or disable one listener

```sh
# Custom ports
rusty-conduit --http-port 8080 --socks-port 1081

# HTTP/HTTPS only (disable SOCKS with 0)
rusty-conduit --socks-port 0

# SOCKS4/4a/5 only (disable HTTP with 0)
rusty-conduit --http-port 0
```

### Serve HTTP/HTTPS and SOCKS on one port

Set both ports to the same non-zero number — `rusty-conduit` automatically peeks at the first byte of each connection to route HTTP, SOCKS4/4a, and SOCKS5 on one port:

```sh
rusty-conduit --http-port 8888 --socks-port 8888
```

### Save traffic logs to files

```sh
rusty-conduit \
  --http-access-log access.log \
  --socks-log socks.log
```

### Watch live traffic in your terminal

Set `--log-level DEBUG` to stream HTTP access logs and SOCKS connection logs directly to `stdout` (while server diagnostics stay on `stderr`):

```sh
rusty-conduit --log-level DEBUG
```

### Block private / loopback destinations (or allowlist specific hosts)

Use `--block` or `--allow` (repeatable or comma-separated; together, `--allow` entries are exceptions to `--block`) with IPs, CIDRs, exact FQDNs, or `*.domain` wildcards:

```sh
# Block loopback and RFC 1918 LAN ranges
rusty-conduit --block 127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16

# Allow only specific domains
rusty-conduit --allow example.com,*.example.org
```

### Listen on your LAN

```sh
rusty-conduit --bind 0.0.0.0
```

> **Note:** v1 has **no proxy authentication**. Binding to a non-loopback address prints a security notice at startup — restrict access with your firewall and keep `--max-connections` bounded (see [Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md)).

### Check a configuration before starting

```sh
rusty-conduit validate --config proxy.toml
```

---

## 5. Where to Go Next

- **[Advanced User Guide](USER-GUIDE-ADVANCED.md):** TOML config files, environment variables, HTTPS MITM inspection, `--socks-block-web`, JSON logs, and full CLI reference.
- **[MITM CA Guide](MITM-CA-GUIDE.md):** Step-by-step instructions to generate and trust your own private CA.
- **[Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md):** Memory/CPU budgets, `ulimit -n` math, and connection/cache caps.
- **[Docker Guide](DOCKER-GUIDE.md):** Running in containers, with `docker run` and Compose examples.
