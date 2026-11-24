# rusty-conduit

**A fast, lightweight, zero-caching HTTP/HTTPS and SOCKS4/4a/5 proxy server in a single Rust binary — with optional pure-Rust HTTPS MITM inspection.**

[![Release](https://img.shields.io/github/v/release/jpbaking/rusty-conduit)](https://github.com/jpbaking/rusty-conduit/releases)
[![License](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](LICENSE)

---

## Why rusty-conduit?

- **One binary, all protocols:** Serves plain HTTP forwarding, HTTPS `CONNECT`, SOCKS4, SOCKS4a, and SOCKS5 on separate ports (`3128` / `1080`) or **auto-multiplexed on a single port**.
- **Strictly zero caching:** Never caches HTTP responses, DNS lookups, or upstream connections, and streams bodies without full buffering. The **only** in-memory cache is for dynamically signed MITM leaf certificates — strictly bounded by TTL and max-entry caps.
- **Pure-Rust TLS, zero OpenSSL:** Built on `rustls` + `ring`, `rcgen`, and `p12-keystore`. Identical single-binary builds on **Linux, macOS (Apple Silicon & Intel), and Windows (`amd64` & `arm64`)**.
- **Opt-in HTTPS MITM without surprises:** OFF by default and **never auto-generates a CA**. Bring your own PEM or PKCS#12 CA, inspect HTTP/1.1 & HTTP/2 traffic, and optionally block web traffic over SOCKS (`--socks-block-web-in-mitm`).
- **Destination allowlist / blocklist:** Restrict egress with `--allow` or `--block` (IP, CIDR, exact FQDN, or `*.domain` wildcards), checking both hostnames and resolved IPs before connecting.
- **Survives long silent connections:** No idle timeout on established tunnels, plus TCP keepalive on client and upstream sockets (default `60s` idle / `10s` × `5` probes) so NAT, firewalls, and VPNs don't drop agents waiting on slow responses.
- **Built for operators:** `CLI > env > TOML` configuration, 6 diagnostic log levels, structured `text` (logfmt) or `json` (NDJSON) access/SOCKS logs, graceful shutdown draining, and built-in connection & cert-cache caps (~6 MB idle RSS).

---

## Quick Start

### 1. Download or install

Download a prebuilt package for Linux, macOS, or Windows (`amd64` / `arm64`) from **[Releases](https://github.com/jpbaking/rusty-conduit/releases)**, run with Docker, or build from source with Rust:

```sh
# Docker (multi-arch: linux/amd64, linux/arm64; binds 0.0.0.0 inside container)
docker run --rm -p 127.0.0.1:3128:3128 -p 127.0.0.1:1080:1080 ghcr.io/jpbaking/rusty-conduit:latest

# Or build from source
cargo install --path .
```

### 2. Run

```sh
# Defaults: 127.0.0.1:3128 (HTTP/HTTPS) and 127.0.0.1:1080 (SOCKS4/4a/5), MITM OFF
rusty-conduit
```

### 3. Test

```sh
curl -x http://127.0.0.1:3128 http://example.com/              # Plain HTTP
curl -x http://127.0.0.1:3128 https://example.com/             # Opaque HTTPS CONNECT
curl --socks5-hostname 127.0.0.1:1080 https://example.com/     # SOCKS5 (remote DNS)
curl --socks4a 127.0.0.1:1080 http://example.com/              # SOCKS4a (remote DNS)
```

Want a single port for everything, or live traffic logs?

```sh
rusty-conduit --http-port 8888 --socks-port 8888 --log-level DEBUG
```

---

## Documentation

| Guide | What's inside |
| :--- | :--- |
| **[Basic User Guide](docs/USER-GUIDE-BASIC.md)** | Installing, connecting browsers/CLI tools, common flags, single-port mode, and watching logs |
| **[Advanced User Guide](docs/USER-GUIDE-ADVANCED.md)** | Full CLI/env/TOML reference, HTTPS MITM inspection, `--socks-block-web-in-mitm`, and JSON logging |
| **[Performance, Footprint & Sizing](docs/PERFORMANCE-AND-SIZING.md)** | CPU/RAM footprint (~6 MB idle, ~25 KB/tunnel), `ulimit -n` math, concurrent user sizing, and resource caps |
| **[MITM CA Guide](docs/MITM-CA-GUIDE.md)** | Generating your own PEM or PKCS#12 CA with `openssl` and trusting it on Linux, macOS, Windows, and Firefox |
| **[Master Plan](docs/MASTER-PLAN.md)** | v1 architecture and design specification |
| **[Future Plans](FUTURE-PLANS.md)** | Post-v1 roadmap (proxy auth, SOCKS5 `UDP ASSOCIATE`, MITM bypass lists, and more) |

---

## License

BSD-3-Clause — see [LICENSE](LICENSE).
