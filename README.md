# rusty-conduit

A lightweight, cross-platform proxy server written in Rust. One binary covers:

- an **HTTP/HTTPS proxy**: plain HTTP forwarding and `CONNECT` tunnels,
- a **SOCKS4, SOCKS4a, and SOCKS5 proxy**: TCP `CONNECT` to IPv4, IPv6, or domain targets,
- optional **HTTPS MITM inspection**, using a CA you supply, which logs every decrypted request,
- **single-port multiplexing**, where HTTP and SOCKS share one port,
- structured **access and SOCKS logs** in logfmt or NDJSON, with six diagnostic log levels.

It uses a pure-Rust TLS stack (`rustls` + `ring`, `rcgen`, `p12-keystore`), with no OpenSSL
and no native C libraries. It builds the same way on Linux, macOS, and Windows.

The design is documented in [docs/MASTER-PLAN.md](docs/MASTER-PLAN.md). Features left out of
v1 (authentication, UDP, MITM bypass lists, and more) are tracked in
[FUTURE-PLANS.md](FUTURE-PLANS.md).

---

## Zero-caching policy

rusty-conduit is a pass-through proxy. It does **not** cache anything except one thing:

| What | Cached? |
| :--- | :--- |
| HTTP responses (plain or decrypted MITM) | **Never.** Every request goes to the origin, whatever `Cache-Control`/`Expires` say. |
| DNS lookups | **Never.** Each connection resolves through the OS resolver. |
| Upstream connections | **Never pooled.** Each forwarded request or tunnel dials a fresh connection. |
| Request/response bodies | **Never buffered in full.** Bodies are streamed frame by frame. |
| MITM leaf certificates | **Yes, the only cache.** The cache is in memory and bounded by `--mitm-cert-ttl`, which defaults to `1h`. Each entry is checked for expiry on every read, so an expired certificate is never served. A background reaper evicts expired entries every `clamp(ttl/4, 5s, 60s)`. Entry count is capped by `--mitm-cert-cache-max-entries`. |

---

## Installation

You need a Rust toolchain (stable, edition 2021). Get one from <https://rustup.rs>.

```sh
git clone <this repository> rusty-conduit
cd rusty-conduit
cargo build --release
# binary: target/release/rusty-conduit   (rusty-conduit.exe on Windows)
```

Or install it into `~/.cargo/bin`:

```sh
cargo install --path .
```

Run the checks used by the project:

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The integration tests in `tests/` start real servers on ephemeral loopback ports against
local mock upstreams and ephemeral CAs. They need no internet access.

---

## Quick start

```sh
# HTTP/HTTPS proxy on 127.0.0.1:3128 and SOCKS on 127.0.0.1:1080 (the defaults)
rusty-conduit

curl -x http://127.0.0.1:3128 http://example.com/
curl -x http://127.0.0.1:3128 https://example.com/          # CONNECT tunnel (opaque)
curl --socks5-hostname 127.0.0.1:1080 https://example.com/  # SOCKS5, remote DNS
curl --socks4a 127.0.0.1:1080 http://example.com/           # SOCKS4a
```

Press `Ctrl+C` (or send `SIGTERM` on Unix) to shut down. The server stops accepting
connections, drains in-flight connections for up to 5 seconds, stops the certificate
reaper, and flushes the log files.

---

## Configuration

Settings come from four sources. When a setting appears in more than one, the higher one wins:

1. **CLI flags** (highest)
2. **Environment variables** (`RUSTY_CONDUIT_*`)
3. **TOML config file** (`--config <PATH>`)
4. **Built-in defaults**

### Reference

| CLI flag | Environment variable | TOML key | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `--config <PATH>` | `RUSTY_CONDUIT_CONFIG` | — | *(none)* | Optional TOML configuration file |
| `--bind <IP>` | `RUSTY_CONDUIT_BIND` | `bind` | `127.0.0.1` | IPv4 or IPv6 address for all listeners |
| `--http-port <PORT>` | `RUSTY_CONDUIT_HTTP_PORT` | `http_port` | `3128` | HTTP/HTTPS proxy port; `0` disables it |
| `--socks-port <PORT>` | `RUSTY_CONDUIT_SOCKS_PORT` | `socks_port` | `1080` | SOCKS4/4a/5 proxy port; `0` disables it |
| `--max-connections <N>` | `RUSTY_CONDUIT_MAX_CONNECTIONS` | `max_connections` | `4096` | Maximum concurrent client connections across all listeners. Excess connections are closed immediately. `0` means unlimited |
| `--mitm-ca-cert <PATH>` | `RUSTY_CONDUIT_MITM_CA_CERT` | `mitm.ca_cert` | *(none)* | PEM CA certificate. With `--mitm-ca-key`, this enables MITM |
| `--mitm-ca-key <PATH>` | `RUSTY_CONDUIT_MITM_CA_KEY` | `mitm.ca_key` | *(none)* | PEM CA private key (PKCS#8, PKCS#1 RSA, SEC1 EC, or encrypted PKCS#8) |
| `--mitm-ca-p12 <PATH>` | `RUSTY_CONDUIT_MITM_CA_P12` | `mitm.ca_p12` | *(none)* | PKCS#12 (`.p12`/`.pfx`) CA bundle. Setting it enables MITM |
| `--mitm-ca-passphrase <STR>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE` | `mitm.ca_passphrase` | *(none)* | Passphrase for the PKCS#12 bundle or encrypted PEM key. Prefer the env var or a file |
| `--mitm-ca-passphrase-file <PATH>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE` | `mitm.ca_passphrase_file` | *(none)* | File holding the passphrase. A trailing `\n` or `\r\n` is trimmed |
| `--mitm-cert-ttl <DURATION>` | `RUSTY_CONDUIT_MITM_CERT_TTL` | `mitm.cert_ttl` | `1h` | How long a generated leaf certificate stays in memory, e.g. `30m`, `1h`, `3600s`. The minimum is `10s` |
| `--mitm-cert-cache-max-entries <N>` | `RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES` | `mitm.cert_cache_max_entries` | `10000` | Maximum cached leaf certificates. At the cap, expired entries are removed first, then the oldest. `0` means unlimited |
| `--socks-block-web-in-mitm` | `RUSTY_CONDUIT_SOCKS_BLOCK_WEB_IN_MITM` | `mitm.socks_block_web` | `false` | When MITM is on, reject HTTP and HTTPS traffic sent over SOCKS |
| `--log-level <LEVEL>` | `RUSTY_CONDUIT_LOG_LEVEL` | `logging.level` | `INFO` | `QUIET`, `FATAL`, `ERROR`, `INFO`, `DEBUG`, `VERBOSE` (case-insensitive) |
| `--log-format <FMT>` | `RUSTY_CONDUIT_LOG_FORMAT` | `logging.format` | `text` | Format of the access and SOCKS logs: `text` (logfmt) or `json` (NDJSON) |
| `--http-access-log <PATH>` | `RUSTY_CONDUIT_HTTP_ACCESS_LOG` | `logging.http_access_log` | *(none)* | File for the HTTP/HTTPS access log (opened in append mode) |
| `--socks-log <PATH>` | `RUSTY_CONDUIT_SOCKS_LOG` | `logging.socks_log` | *(none)* | File for the SOCKS connection log (opened in append mode) |
| `-h`, `--help` / `-V`, `--version` | — | — | — | Print help or version and exit |

Notes:

- **Durations** use [humantime](https://docs.rs/humantime) syntax, for example `90s`, `30m`,
  `1h 30m`, or `2h`.
- **`RUSTY_CONDUIT_SOCKS_BLOCK_WEB_IN_MITM`** accepts boolean-like values such as `true`,
  `false`, `1`, `0`, `yes`, `no`, `on`, and `off`. If you set it through the CLI or the
  environment, that value overrides `mitm.socks_block_web` in the TOML file.
- **MITM is on** exactly when a complete CA source is configured: either `--mitm-ca-cert` plus
  `--mitm-ca-key`, or `--mitm-ca-p12`. rusty-conduit **never generates a CA**. See
  [docs/MITM-CA-GUIDE.md](docs/MITM-CA-GUIDE.md) for how to create and trust one.

### Startup validation

The following problems stop startup with a `FATAL` message and exit code `1`:

- `http_port` and `socks_port` are both `0`.
- `--mitm-ca-p12` is combined with `--mitm-ca-cert` or `--mitm-ca-key`, so the CA source is ambiguous.
- Only one of `--mitm-ca-cert` or `--mitm-ca-key` is given.
- Both `--mitm-ca-passphrase` and `--mitm-ca-passphrase-file` are given.
- `--mitm-cert-ttl` is shorter than `10s`.
- The CA cannot be read or parsed, is expired or not yet valid, or does not match its private key.
  A wrong PKCS#12 passphrase also fails here. Keys in the legacy OpenSSL
  `Proc-Type: 4,ENCRYPTED` format are rejected, and the error suggests how to convert them.
- A listener cannot bind, or an access log file cannot be opened.

These conditions only produce `INFO` notices:

- The bind address is not loopback (for example `0.0.0.0`). This prints a **security notice**:
  v1 has **no authentication**, so anyone who can reach the address can use the proxy.
- `--mitm-ca-passphrase` was given on the command line, where other users and your shell
  history can see it.
- `--socks-block-web-in-mitm` is set while MITM is off. The flag then has no effect.

### Example TOML file

```toml
# rusty-conduit.toml — every key is optional; unknown keys are rejected.
bind = "127.0.0.1"
http_port = 3128
socks_port = 1080
max_connections = 4096     # 0 = unlimited

[mitm]
# Either a PEM pair...
ca_cert = "/etc/rusty-conduit/ca.pem"
ca_key  = "/etc/rusty-conduit/ca.key"
# ...or a PKCS#12 bundle (not both):
# ca_p12 = "/etc/rusty-conduit/ca.p12"
# ca_passphrase_file = "/etc/rusty-conduit/ca.pass"
cert_ttl = "30m"
cert_cache_max_entries = 10000     # 0 = unlimited
socks_block_web = true

[logging]
level = "INFO"
format = "json"
http_access_log = "/var/log/rusty-conduit/access.log"
socks_log = "/var/log/rusty-conduit/socks.log"
```

```sh
rusty-conduit --config rusty-conduit.toml
RUSTY_CONDUIT_LOG_LEVEL=DEBUG rusty-conduit --config rusty-conduit.toml   # env overrides file
rusty-conduit --config rusty-conduit.toml --http-port 8080                # CLI overrides both
```

---

## Usage examples

### Ports and multiplexing

```sh
# Separate ports (default): HTTP on 3128, SOCKS on 1080
rusty-conduit

# One port for everything: when both ports are equal (and non-zero), each connection is
# classified by its first byte: 0x04 → SOCKS4/4a, 0x05 → SOCKS5, 'A'..'Z' → HTTP.
# Connections that start with any other byte are closed.
rusty-conduit --http-port 8888 --socks-port 8888
curl -x http://127.0.0.1:8888 http://example.com/
curl --socks5-hostname 127.0.0.1:8888 https://example.com/

# SOCKS only (HTTP listener disabled)
rusty-conduit --http-port 0

# HTTP only (SOCKS listener disabled)
rusty-conduit --socks-port 0

# Listen on all IPv6 (and, depending on the OS, IPv4) interfaces — prints a security notice
rusty-conduit --bind ::
```

### HTTPS MITM inspection

```sh
# PEM certificate + key
rusty-conduit --mitm-ca-cert ca.pem --mitm-ca-key ca.key --http-access-log access.log

# PKCS#12 bundle; passphrase from the environment (preferred) or a file
RUSTY_CONDUIT_MITM_CA_PASSPHRASE='…' rusty-conduit --mitm-ca-p12 ca.p12
rusty-conduit --mitm-ca-p12 ca.p12 --mitm-ca-passphrase-file ca.pass

# Clients must trust the CA (see docs/MITM-CA-GUIDE.md):
curl -x http://127.0.0.1:3128 --cacert ca.pem https://example.com/
```

How MITM works:

- MITM applies only to HTTP-proxy `CONNECT` tunnels. SOCKS stays a transparent TCP tunnel.
- For each tunnel, rusty-conduit gives the client a leaf certificate it signed for the
  requested host. The leaf uses the SNI name, or the `CONNECT` host when there is no SNI.
- It then opens a **new, verified** TLS connection to the real origin, using the OS
  certificate store.
- HTTP/1.1 and HTTP/2 are both decrypted, and protocol upgrades such as WebSocket pass through.
- Every decrypted request is logged with `proto=https mitm=true` and the full `https://` URL.
- Each leaf has a fresh ECDSA P-256 key, a random 128-bit serial, and a single SAN (a DNS name
  or an IP address).
- A leaf is valid from `now − 5m` to `now + TTL + 5m`. The extra 5 minutes at each end cover
  client clock skew and certificates handed out just before their cache entry expires.

Add `--socks-block-web-in-mitm` to stop clients from getting around MITM logging by sending
web traffic over SOCKS:

```sh
rusty-conduit --mitm-ca-cert ca.pem --mitm-ca-key ca.key --socks-block-web-in-mitm
```

With the flag on, rusty-conduit inspects the **first** chunk the client sends on each SOCKS
session. It closes the session without forwarding that chunk, and logs
`status=blocked_web_in_mitm`, when the chunk is either:

- **plain HTTP**: it starts with `GET `, `POST `, `HEAD `, `PUT `, `DELETE `, `OPTIONS `,
  `PATCH `, `CONNECT `, or `TRACE `, or
- **a TLS `ClientHello` for HTTPS**: it advertises ALPN `http/1.1` or `h2`, or it targets port
  `443` without advertising a non-HTTP ALPN.

Everything else passes through unchanged. Data from the server is always forwarded right away,
so protocols where the server speaks first (SSH, SMTP, Postgres, …) keep working.

---

## Logging

### Levels

Diagnostic messages go to **stderr**. At `DEBUG` and `VERBOSE`, access and SOCKS records are
**also** copied to **stdout**, so you can pipe them separately.

| Level | Diagnostics on stderr | Access / SOCKS records |
| :--- | :--- | :--- |
| `QUIET` | none | log files only (if configured) |
| `FATAL` | unrecoverable startup errors | log files only |
| `ERROR` | `FATAL` + runtime/connection errors | log files only |
| `INFO` *(default)* | `ERROR` + startup banner, config summary, security notices, shutdown | log files only |
| `DEBUG` | `INFO` + per-connection lifecycle, rejected handshakes | log files **and stdout** |
| `VERBOSE` | `DEBUG` + low-level details such as certificate reaper statistics | log files **and stdout** |

Diagnostic line format: `2026-11-22T13:30:01.123Z INFO    <message>`.

### Access and SOCKS log formats

Each record is one line. The `text` format (the default, logfmt):

```text
ts=2026-11-22T13:30:01.123Z kind=http client=127.0.0.1:51234 proto=https mitm=true method=GET target=https://example.com/api/v1 status=200 bytes_in=142 bytes_out=4096 duration_ms=84
ts=2026-11-22T13:30:02.456Z kind=socks client=127.0.0.1:51240 proto=socks5 cmd=CONNECT target=github.com:22 status=ok bytes_in=1280 bytes_out=8192 duration_ms=1520
```

The `json` format (`--log-format json`, NDJSON) has the same fields in the same order:

```json
{"ts":"2026-11-22T13:30:01.123Z","kind":"http","client":"127.0.0.1:51234","proto":"https","mitm":true,"method":"GET","target":"https://example.com/api/v1","status":200,"bytes_in":142,"bytes_out":4096,"duration_ms":84}
```

HTTP access fields (`kind=http`), one record per forwarded request or `CONNECT` tunnel:

| Field | Meaning |
| :--- | :--- |
| `ts` | Start time (RFC 3339 UTC, milliseconds) |
| `client` | Client socket address |
| `proto` | `http` for plain forwarding, `https` for `CONNECT` and decrypted MITM requests |
| `mitm` | `true` for a request decrypted inside a MITM tunnel |
| `method`, `target` | Request method and URL (`host:port` for an opaque `CONNECT`) |
| `status` | HTTP status returned to the client (for example `502` if the upstream is unreachable) |
| `bytes_in`, `bytes_out` | Body or tunnel bytes from the client, and to the client |
| `duration_ms` | Time until the response body finished or the tunnel closed |

SOCKS fields (`kind=socks`), one record per session:

| Field | Meaning |
| :--- | :--- |
| `proto` | `socks4`, `socks4a`, or `socks5` |
| `cmd` | `CONNECT`, `BIND`, `UDP_ASSOCIATE`, … (`-` if the request was never parsed) |
| `target` | `host:port` (`[v6]:port` for IPv6) |
| `status` | `ok`, `blocked_web_in_mitm`, `command_not_supported`, `address_type_not_supported`, `no_acceptable_auth`, `connection_refused`, `host_unreachable`, `network_unreachable`, or `general_failure` |
| `bytes_in`, `bytes_out` | Payload bytes from and to the client (handshake bytes are not counted) |

A plain `CONNECT` tunnel (MITM off) is logged once, when it closes, as `proto=https
mitm=false method=CONNECT`.

---

## Performance, Footprint & Concurrency

### Memory and CPU

The **measured** figures come from one release build on 64-bit Linux, using idle `CONNECT`
tunnels over loopback after 1 KiB of traffic each. They count process RSS and exclude kernel
socket buffers. The **estimated** figures were not measured, so check them under your own load.

| Item | RAM | CPU | Basis |
| :--- | :--- | :--- | :--- |
| Idle process | about 6 MB | about 0 | Measured |
| Opaque tunnel (`CONNECT` or SOCKS) | about 25 KB each | Low; bytes are copied without inspection | Measured for `CONNECT`; SOCKS estimated |
| Plain HTTP request | about 25–50 KB while active | Low; adds header parsing | Estimated |
| MITM tunnel | about 50–150 KB each | Highest; TLS on both sides of every byte | Estimated |
| Cached leaf certificate | about 3–5 KB each, so 30–50 MB at the default cap | One ECDSA P-256 signature per cache miss | Estimated |

Throughput is usually limited by the network or, with MITM, by CPU. The async runtime uses one
worker thread per CPU core.

### File descriptors

Each proxied connection uses **2 file descriptors**: one for the client and one for the
upstream. A MITM session can briefly use more while it dials extra upstream connections. Allow
for this when sizing:

```text
ulimit -n  >=  (2 × max_connections) + 64
```

The default cap of `4096` needs a limit of at least `8256`. Many Linux shells default to `1024`,
which supports only about 480 connections, so either raise the limit or lower the cap:

```sh
ulimit -n 8256 && rusty-conduit
rusty-conduit --max-connections 480   # stay within a 1024 descriptor limit
```

Under systemd, set `LimitNOFILE=8256` in the service unit. If descriptors run out, `accept`
fails, an `ERROR` is logged, and the listener retries after 100 ms.

### Sizing rules of thumb

These assume about 10 concurrent connections per active user, which varies widely with the
client and workload.

| Deployment | `--max-connections` | `ulimit -n` | Approx. RAM | Approx. active users |
| :--- | :--- | :--- | :--- | :--- |
| Personal or single device | `256` | `1024` | about 15 MB (opaque), up to 60 MB (MITM) | 1–25 |
| Home or small team | `1024` | `2112` | about 35 MB (opaque), up to 200 MB (MITM) | 25–100 |
| Default | `4096` | `8256` | about 110 MB (opaque), up to 700 MB (MITM) | 100–400 |

### How the caps behave

- **`--max-connections`** is one pool shared by all listeners. When it is full, a new connection
  is accepted and closed immediately, with no reply and no queue, and a `DEBUG` line is logged.
  Existing connections are not affected, and a slot is freed as soon as a connection ends.
- **`--mitm-cert-cache-max-entries`** is a strict bound on stored certificates. When a new host
  arrives at the cap, expired entries are removed first. If the cache is still full, the entries
  closest to expiry are evicted, and this is logged at `VERBOSE`. Cache hits are unaffected, and
  an evicted host is simply signed again on its next connection.
- **`0`** disables either cap. Avoid this on untrusted networks, because memory and descriptor
  use then have no upper bound from rusty-conduit.

---

## Protocol support (v1)

| Protocol | Supported | Not in v1 |
| :--- | :--- | :--- |
| HTTP proxy | Absolute-form `http://` forwarding with RFC 9110 hop-by-hop header stripping; `CONNECT` tunnels | `Proxy-Authorization` |
| SOCKS4 / 4a | `CONNECT` (IPv4; SOCKS4a domain names resolved by the proxy). The user ID is recorded but not authenticated | `BIND` (rejected with `0x5B`) |
| SOCKS5 | No-auth `CONNECT` to IPv4, IPv6, and domain targets | Username/password auth; `BIND` and `UDP ASSOCIATE` (rejected with `0x07`) |

See [FUTURE-PLANS.md](FUTURE-PLANS.md) for the post-v1 roadmap.

---

## License

BSD-3-Clause — see [LICENSE](LICENSE).
