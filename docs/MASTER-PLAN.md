# Rusty Conduit — Master Plan

> **Status:** Approved Design Blueprint (v1)  
> **Target Platforms:** Linux, Windows, macOS (cross-platform single binary, pure-Rust TLS stack)  
> **Deferred / Post-v1 Roadmap:** See [FUTURE-PLANS.md](../FUTURE-PLANS.md)

---

## 1. Overview & Core Principles

`rusty-conduit` is a lightweight, cross-platform Rust CLI proxy server supporting **HTTP/HTTPS** and **SOCKS4/SOCKS4a/SOCKS5** with optional **HTTPS MITM inspection**.

### Non-Negotiable Principles
1. **Zero Caching (Except MITM Leaf Certificates):**
   - **No** HTTP response caching.
   - **No** custom DNS caching (all resolution delegates directly to the OS/Tokio resolver).
   - **No** full-body request/response buffering (strictly streaming).
   - The **only** in-memory cache is for dynamically generated MITM leaf TLS certificates, strictly governed by a configurable TTL, lazy read-time expiration checks, and a periodic background reaper task.
2. **MITM is Opt-In & Never Auto-Generates a CA:**
   - MITM mode is **OFF** by default (no private CA configured).
   - The application will **never** auto-generate a root CA keypair. Users supply their own PEM or PKCS#12 CA; documentation (`docs/MITM-CA-GUIDE.md`) will guide users on creating and trusting one across Linux, macOS, and Windows.
3. **Cross-Platform Purity:**
   - Pure-Rust TLS (`rustls` + `rcgen` + `p12-keystore`) with **zero OpenSSL / native C-library dependencies**, ensuring straightforward builds and portable binaries on Windows, Linux, and macOS.

---

## 2. Locked-In Design Decisions (v1)

| Area | Decision |
| :--- | :--- |
| **Ports & Multiplexing** | Separate ports by default (`3128` HTTP/HTTPS, `1080` SOCKS4/5). If both are configured to the same port (`> 0`), automatically enable single-port first-byte protocol multiplexing. Setting either port to `0` disables that listener. |
| **MITM Scope** | HTTP/HTTPS `CONNECT` tunnels only. SOCKS4/5 remains a pure transparent L4 TCP tunnel, with an optional `--socks-block-web-in-mitm` flag to reject HTTP/HTTPS traffic over SOCKS when MITM mode is active. |
| **CA Formats & TLS** | Support both **PEM** (`cert` + `key`) and **PKCS#12** (`.p12`/`.pfx` + passphrase). Pure-Rust `rustls` + `rcgen`. Default cert cache TTL `1h` with a `5m` clock-skew (`not_before`) and expiry grace (`not_after`) buffer. |
| **SOCKS Scope & Auth** | TCP only: SOCKS4, SOCKS4a (remote DNS), and SOCKS5 `CONNECT` (IPv4, IPv6, Domain). No auth in v1 (prominent startup warning if binding to a non-loopback address). Auth & UDP deferred to [FUTURE-PLANS.md](../FUTURE-PLANS.md). |
| **Configuration** | CLI flags (`clap`) + environment variables (`RUSTY_CONDUIT_*`) + optional TOML config file (`--config`). Precedence: **CLI > Env > Config File > Defaults**. |
| **Logging** | 6 custom levels: `QUIET`, `FATAL`, `ERROR`, `INFO` (default), `DEBUG`, `VERBOSE`. Access/SOCKS logs default to human-readable `text` (logfmt) with optional `--log-format json` (NDJSON). `DEBUG` and `VERBOSE` dump access/SOCKS logs to `stdout` (in addition to configured log files). |

---

## 3. Configuration Interface

### Precedence Order
1. CLI flags (highest priority)
2. Environment variables (`RUSTY_CONDUIT_*`)
3. Optional TOML config file (`--config <path>`)
4. Built-in defaults

### Configuration Reference

| CLI Flag | Env Var | TOML Key | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `--config <PATH>` | `RUSTY_CONDUIT_CONFIG` | — | *(none)* | Optional TOML configuration file path |
| `--bind <IP>` | `RUSTY_CONDUIT_BIND` | `bind` | `127.0.0.1` | IPv4 or IPv6 address to bind listeners to |
| `--http-port <PORT>` | `RUSTY_CONDUIT_HTTP_PORT` | `http_port` | `3128` | HTTP/HTTPS proxy port (`0` to disable) |
| `--socks-port <PORT>` | `RUSTY_CONDUIT_SOCKS_PORT` | `socks_port` | `1080` | SOCKS4/5 proxy port (`0` to disable) |
| `--mitm-ca-cert <PATH>` | `RUSTY_CONDUIT_MITM_CA_CERT` | `mitm.ca_cert` | *(none)* | Path to PEM-encoded CA certificate (enables MITM with `--mitm-ca-key`) |
| `--mitm-ca-key <PATH>` | `RUSTY_CONDUIT_MITM_CA_KEY` | `mitm.ca_key` | *(none)* | Path to PEM-encoded CA private key |
| `--mitm-ca-p12 <PATH>` | `RUSTY_CONDUIT_MITM_CA_P12` | `mitm.ca_p12` | *(none)* | Path to PKCS#12 (`.p12`/`.pfx`) CA bundle (enables MITM) |
| `--mitm-ca-passphrase <STR>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE` | `mitm.ca_passphrase` | *(none)* | Passphrase for PKCS#12 or encrypted PEM key *(prefer env var or file)* |
| `--mitm-ca-passphrase-file <PATH>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE` | `mitm.ca_passphrase_file` | *(none)* | File containing the CA passphrase (trimmed of trailing newline) |
| `--mitm-cert-ttl <DURATION>` | `RUSTY_CONDUIT_MITM_CERT_TTL` | `mitm.cert_ttl` | `1h` | In-memory TTL for generated MITM leaf certs (e.g. `30m`, `1h`, `3600s`) |
| `--socks-block-web-in-mitm` | `RUSTY_CONDUIT_SOCKS_BLOCK_WEB_IN_MITM` | `mitm.socks_block_web` | `false` | When MITM is ON, reject HTTP/HTTPS traffic over SOCKS4/5 |
| `--log-level <LEVEL>` | `RUSTY_CONDUIT_LOG_LEVEL` | `logging.level` | `INFO` | `QUIET`, `FATAL`, `ERROR`, `INFO`, `DEBUG`, `VERBOSE` (case-insensitive) |
| `--log-format <FMT>` | `RUSTY_CONDUIT_LOG_FORMAT` | `logging.format` | `text` | `text` (logfmt) or `json` (NDJSON) for access/SOCKS logs |
| `--http-access-log <PATH>` | `RUSTY_CONDUIT_HTTP_ACCESS_LOG` | `logging.http_access_log` | *(none)* | Optional file path for HTTP/HTTPS access logs |
| `--socks-log <PATH>` | `RUSTY_CONDUIT_SOCKS_LOG` | `logging.socks_log` | *(none)* | Optional file path for SOCKS4/5 connection logs |

### Startup Validation Rules
- If both `http_port == 0` and `socks_port == 0`: exit with `FATAL`.
- If `--mitm-ca-p12` is combined with `--mitm-ca-cert` or `--mitm-ca-key`: exit with `FATAL` (ambiguous CA source).
- If only one of `--mitm-ca-cert` or `--mitm-ca-key` is provided: exit with `FATAL`.
- If both `--mitm-ca-passphrase` and `--mitm-ca-passphrase-file` are provided: exit with `FATAL`.
- If `--mitm-cert-ttl` is `< 10s`: exit with `FATAL`.
- If `--socks-block-web-in-mitm` is set while MITM is OFF: log an `INFO` notice that the flag is inactive because MITM is disabled.
- If `--bind` is not a loopback address (`!ip.is_loopback()`, e.g. `0.0.0.0` or LAN IP): emit a prominent `INFO` security notice noting that v1 runs without authentication.
- If `--mitm-ca-passphrase` was passed via CLI argument: emit an `INFO` notice recommending `RUSTY_CONDUIT_MITM_CA_PASSPHRASE` or `--mitm-ca-passphrase-file` instead.

---

## 4. System Architecture & Module Layout

```text
rusty-conduit/
├── Cargo.toml
├── README.md
├── FUTURE-PLANS.md
├── docs/
│   ├── MASTER-PLAN.md
│   └── MITM-CA-GUIDE.md
├── src/
│   ├── main.rs              # Entrypoint, signal handling, orchestration
│   ├── config.rs            # CLI (clap) + env + TOML loading & validation
│   ├── logging.rs           # 6-level diagnostic logger + HTTP/SOCKS log sinks
│   ├── server.rs            # Listener lifecycle (separate vs. multiplexed) & graceful shutdown
│   ├── mux.rs               # Single-port first-byte protocol sniffer
│   ├── net.rs               # Shared TCP dialing, bidirectional relay with byte counters
│   ├── http/
│   │   ├── mod.rs           # HTTP connection dispatcher
│   │   ├── forward.rs       # Plain HTTP forward proxy (RFC 9110, streaming)
│   │   ├── tunnel.rs        # Non-MITM HTTPS CONNECT opaque TCP tunnel
│   │   └── mitm_tunnel.rs   # MITM HTTPS CONNECT TLS termination + re-encryption
│   ├── socks/
│   │   ├── mod.rs           # SOCKS4/4a/5 protocol dispatcher
│   │   ├── v4.rs            # SOCKS4 & SOCKS4a handshake + CONNECT
│   │   ├── v5.rs            # SOCKS5 handshake (no-auth) + CONNECT (IPv4/IPv6/Domain)
│   │   └── web_guard.rs     # First-chunk HTTP/TLS-ALPN detector for --socks-block-web-in-mitm
│   └── mitm/
│       ├── mod.rs           # MITM context & rustls ServerConfig / ClientConfig builders
│       ├── ca.rs            # PEM & PKCS#12 CA loader + validation
│       ├── signer.rs        # rcgen dynamic leaf cert generation (SANs, skew/grace window)
│       └── cache.rs         # TTL CertCache + single-flight dedup + background reaper
└── tests/
    ├── http_proxy.rs        # End-to-end plain HTTP & HTTPS CONNECT tests
    ├── mitm_proxy.rs        # End-to-end MITM cert generation, TTL expiry, and inspection tests
    ├── socks_proxy.rs       # End-to-end SOCKS4, SOCKS4a, SOCKS5 & web-blocking tests
    └── mux_port.rs          # Single-port multiplexing & port-disable tests
```

### Listener & Single-Port Multiplexing (`server.rs`, `mux.rs`)
- **Separate Ports (default):**
  - If `http_port > 0` and `http_port != socks_port`: bind HTTP/HTTPS listener on `(bind, http_port)`.
  - If `socks_port > 0` and `socks_port != http_port`: bind SOCKS4/5 listener on `(bind, socks_port)`.
- **Multiplexed Single Port (`http_port == socks_port && http_port > 0`):**
  - Bind one `TcpListener` on `(bind, http_port)`.
  - On each accepted `TcpStream`, call `stream.peek(&mut [0u8; 1])` with a short timeout (e.g. `5s`):
    - `0x04` → dispatch to `socks::v4`
    - `0x05` → dispatch to `socks::v5`
    - `b'A'..=b'Z'` → dispatch to `http`
    - Otherwise → close connection and log at `DEBUG`/`ERROR`.
- **Graceful Shutdown:**
  - Listen for `Ctrl+C` (all platforms) and `SIGTERM` (`#[cfg(unix)]`).
  - Cancel a shared `CancellationToken` to stop accepting new connections, stop the MITM cert-cache reaper, flush open access/SOCKS log files, and drain in-flight connections with a bounded shutdown timeout.

---

## 5. HTTP/HTTPS Proxy Engine (`src/http/`)

### Plain HTTP Forward Proxy (`forward.rs`)
- Served via `hyper` 1.x (`http1` server).
- Rewrites absolute-form request URIs to origin-form when forwarding upstream.
- Strips RFC 9110 hop-by-hop headers (`Proxy-Connection`, `Connection`, `Keep-Alive`, `Proxy-Authenticate`, `Proxy-Authorization`, `TE`, `Trailers`, `Transfer-Encoding`, `Upgrade` unless upgrading).
- Streams request and response bodies directly without buffering or caching.
- Records an HTTP access log entry per request (`proto=http`, `mitm=false`).

### HTTPS `CONNECT` — MITM OFF (`tunnel.rs`)
- Validates `host:port` target from the `CONNECT` authority.
- Connects to upstream TCP `(host, port)`.
- Replies `HTTP/1.1 200 Connection Established\r\n\r\n` (via `hyper::upgrade::on`).
- Runs `tokio::io::copy_bidirectional` with byte counters (`bytes_in`, `bytes_out`) and wall-clock duration.
- Records an HTTP access log entry upon tunnel close (`method=CONNECT`, `mitm=false`).

### HTTPS `CONNECT` — MITM ON (`mitm_tunnel.rs`)
- Replies `HTTP/1.1 200 Connection Established\r\n\r\n` and upgrades the client stream.
- Uses `tokio_rustls::LazyConfigAcceptor` to inspect the client's `ClientHello` (extracting SNI and ALPN):
  - Target hostname for leaf cert = SNI if present, else the `CONNECT` target host/IP.
  - Fetches or generates a leaf `CertifiedKey` via `CertCache::get_or_create(host)`.
  - Completes TLS handshake with the client.
- Establishes an upstream TLS connection via `tokio-rustls` using system/platform root certificates (`rustls-platform-verifier`) and the target SNI.
- Serves decrypted HTTP/1.1 (and HTTP/2 when negotiated) over the client TLS stream and forwards each request over the upstream TLS connection — **streaming with zero caching**.
- Handles protocol upgrades inside MITM (e.g., WebSocket `101 Switching Protocols`) by falling back to raw bidirectional relay after forwarding the upgrade handshake.
- Emits an HTTP access log entry for every decrypted request (`proto=https`, `mitm=true`, method, full `https://host/path`, status code, bytes in/out, latency).

---

## 6. MITM Certificate Authority & TTL Cache (`src/mitm/`)

### CA Loading (`ca.rs`)
- **PEM mode** (`--mitm-ca-cert` + `--mitm-ca-key`): parses PEM cert chain and private key (supports PKCS#8, RSA, ECDSA).
- **PKCS#12 mode** (`--mitm-ca-p12`): parses `.p12`/`.pfx` in pure Rust (`p12-keystore`) using the optional passphrase (from env, file, or flag), extracting the CA certificate and private key.
- **Startup Validation:** Verifies the loaded certificate is parseable, currently valid (not expired), and matches the private key. Fails fast with `FATAL` on any error.

### Dynamic Leaf Signing (`signer.rs`)
- Uses `rcgen` to generate a fresh ECDSA P-256 leaf keypair (fast signing, small certs, universal client support) signed by the loaded CA.
- Sets Subject CN and SAN (`SanType::DnsName` for domains, `SanType::IpAddress` for IP literals).
- Assigns a cryptographically random 128-bit positive serial number.
- **Validity Window (Race-Condition & Clock-Skew Protection):**
  - Cache TTL = `T` (configurable via `--mitm-cert-ttl`, default `1h`).
  - `not_before = now - 5m` (protects against client clock skew).
  - `not_after  = now + T + 5m` (grace buffer so a cert handed out at `T - 1s` never expires mid-handshake or mid-connection).

### In-Memory TTL Cache & Background Reaper (`cache.rs`)
- Backed by `DashMap<String, CacheEntry>` (keyed by lowercase hostname/IP).
- `CacheEntry { key: Arc<CertifiedKey>, expires_at: Instant }` where `expires_at = Instant::now() + ttl`.
- **Single-Flight Deduplication:** Per-host async mutex/notify (`DashMap<String, Arc<tokio::sync::Mutex<()>>>` or `OnceCell` pattern) so 50 simultaneous TLS handshakes to `example.com` sign only **1** leaf certificate instead of 50.
- **Lazy Expiry on Read:** On lookup, if `Instant::now() >= entry.expires_at`, the entry is immediately ignored/replaced so an expired cert is *never* returned even between reaper ticks.
- **Background Reaper Task:**
  - Spawned at startup only when MITM is enabled.
  - Wakes every `reap_interval = clamp(ttl / 4, 5s, 60s)` (or on shutdown signal).
  - Calls `map.retain(|_, entry| now < entry.expires_at)`.
  - Logs eviction count and remaining cache size at `VERBOSE`.

---

## 7. SOCKS4/4a & SOCKS5 Engine (`src/socks/`)

### Supported Protocols (v1)
- **SOCKS4** (`0x04`): `CONNECT` (`0x01`), IPv4 address + port, optional null-terminated user ID (logged as metadata if present, not authenticated).
- **SOCKS4a** (`0x04` with `0.0.0.x` where `x != 0`): reads null-terminated domain name after user ID and resolves remotely via Tokio/OS DNS.
- **SOCKS5** (`0x05`, RFC 1928):
  - Auth negotiation: selects `0x00` (No Authentication Required); rejects if client does not offer `0x00` (`0xFF`).
  - Command: `CONNECT` (`0x01`) only; replies `0x07` (Command not supported) for `BIND` (`0x02`) and `UDP ASSOCIATE` (`0x03`).
  - Address types: IPv4 (`0x01`), Domain (`0x03`), IPv6 (`0x04`).

### Optional Web-Traffic Blocking in MITM Mode (`web_guard.rs`)
When MITM mode is **ON** and `--socks-block-web-in-mitm` is enabled:
- Goal: prevent clients from bypassing HTTP/HTTPS MITM logging by routing web traffic over SOCKS, **without** breaking server-speaks-first protocols (e.g., SSH, SMTP, Postgres).
- Mechanism:
  1. Complete SOCKS handshake and connect upstream normally so server-speaks-first traffic (`upstream -> client`) flows unobstructed.
  2. Before forwarding the **first** `client -> upstream` payload chunk, inspect its leading bytes:
     - **Plaintext HTTP detection:** Starts with an HTTP method prefix (`GET `, `POST `, `HEAD `, `PUT `, `DELETE `, `OPTIONS `, `PATCH `, `CONNECT `, `TRACE `).
     - **HTTPS (TLS) detection:** Starts with a TLS record header (`0x16 0x03 <minor>`) containing a `ClientHello` (`0x01`) where either:
       - the ALPN extension advertises `http/1.1` or `h2`, **or**
       - the target port is `443` (when no non-HTTP ALPN is present).
  3. If matched: do **not** forward the bytes to upstream; immediately close both sockets and record a SOCKS log entry with `status=blocked_web_in_mitm`.
  4. Otherwise: forward the inspected buffer and switch to zero-overhead `copy_bidirectional`.

---

## 8. Logging Architecture (`src/logging.rs`)

### Custom 6-Level Hierarchy
| Level | Diagnostic Output (`stderr`) | HTTP Access & SOCKS Logs |
| :--- | :--- | :--- |
| `QUIET` | None | File only (if path configured) |
| `FATAL` | Unrecoverable startup/crash errors only | File only (if path configured) |
| `ERROR` | `FATAL` + runtime/connection errors | File only (if path configured) |
| `INFO` *(default)* | `ERROR` + startup banner, config summary, security notices, shutdown | File only (if path configured) |
| `DEBUG` | `INFO` + per-connection lifecycle details, MITM cert cache misses | **File (if configured) AND `stdout`** |
| `VERBOSE` | `DEBUG` + protocol handshake details, cert reaper stats, low-level traces | **File (if configured) AND `stdout`** |

> **Stream Separation:** Diagnostic/server logs write to `stderr`, while `DEBUG`/`VERBOSE` access & SOCKS log dumps write to `stdout` so logs can be cleanly piped or redirected.

### Log Sinks & Formatting
- Dedicated async file writers (append mode, buffered, flushed on graceful shutdown) for `--http-access-log` and `--socks-log`.
- **Format 1: `text` (default, logfmt-style)**
  - HTTP/HTTPS access log:
    ```text
    ts=2026-11-22T13:30:01.123Z kind=http client=127.0.0.1:51234 proto=https mitm=true method=GET target=https://example.com/api/v1 status=200 bytes_in=142 bytes_out=4096 duration_ms=84
    ```
  - SOCKS connection log:
    ```text
    ts=2026-11-22T13:30:02.456Z kind=socks client=127.0.0.1:51240 proto=socks5 cmd=CONNECT target=github.com:22 status=ok bytes_in=1280 bytes_out=8192 duration_ms=1520
    ```
- **Format 2: `json` (NDJSON)**
  - Identical fields serialized as one JSON object per line via `serde_json`.

---

## 9. Crate Stack & Cross-Platform Strategy

| Concern | Crate(s) | Rationale |
| :--- | :--- | :--- |
| Async Runtime | `tokio` (`full` minus unused features) | Industry-standard cross-platform async I/O |
| HTTP/1.1 & HTTP/2 | `hyper` 1.x, `hyper-util`, `http`, `http-body-util` | Zero-copy streaming, explicit upgrade support for `CONNECT` |
| TLS (Client & Server) | `rustls`, `tokio-rustls`, `rustls-platform-verifier` | Pure Rust, zero OpenSSL, uses OS root store for upstream verification |
| Cert Generation & CA | `rcgen`, `p12-keystore`, `rustls-pemfile`, `x509-parser` | Pure Rust PEM & PKCS#12 parsing + fast dynamic leaf signing |
| Concurrency | `dashmap`, `tokio-util` (`CancellationToken`) | Lock-free cert cache + clean coordinated shutdown |
| CLI & Config | `clap` (`derive`, `env`), `serde`, `toml`, `humantime` | Clean CLI/env/TOML precedence |
| Logging & Time | `serde_json`, `time` (RFC 3339 timestamps) | Lightweight, deterministic formatting |
| Error Handling | `thiserror`, `anyhow` | Ergonomic typed internal errors + clear CLI exit diagnostics |

### Cross-Platform Targets & CI Matrix
- `x86_64-unknown-linux-gnu` (and `x86_64-unknown-linux-musl` for static Linux binary)
- `x86_64-pc-windows-msvc`
- `aarch64-apple-darwin` & `x86_64-apple-darwin`

---

## 10. Documentation Deliverables

1. **`README.md`**: Quick start, installation, CLI/env/TOML examples, port multiplexing, logging examples.
2. **`docs/MITM-CA-GUIDE.md`**:
   - Generating a private root CA in PEM format (`openssl`) and PKCS#12 format (`.p12`).
   - Trusting the CA on **Linux** (`update-ca-certificates` / `trust anchor`), **macOS** (`security add-trusted-cert` / Keychain), **Windows** (`certutil -addstore Root` / CertMgr), and **Firefox** (`security.enterprise_roots.enabled`).
   - Security best practices (keep the private CA key offline/local, never commit it, set restrictive file permissions, use env var or passphrase file).

---

## 11. Phased Implementation Roadmap

### Phase 1 — Foundation, Config & Logging
- Initialize Cargo project (`rusty-conduit`), CI check skeleton, and module tree.
- Implement `config.rs` (CLI + `RUSTY_CONDUIT_*` env + `--config` TOML + validation rules).
- Implement `logging.rs` (6-level diagnostic logger + async HTTP/SOCKS access log sinks in `text` & `json` + `DEBUG`/`VERBOSE` stdout routing).

### Phase 2 — Server Lifecycle & Port Multiplexer
- Implement `server.rs` (separate listeners, `0` to disable, single-port auto-mux when `http_port == socks_port`, graceful shutdown).
- Implement `mux.rs` (first-byte `peek` dispatcher with timeout).
- Implement `net.rs` (TCP dialer with timeouts + bidirectional byte-counting relay).

### Phase 3 — HTTP/HTTPS Proxy (Non-MITM)
- Implement `http/forward.rs` (streaming plain HTTP proxy, RFC 9110 hop-by-hop handling).
- Implement `http/tunnel.rs` (HTTPS `CONNECT` opaque TCP tunnel).
- Emit structured HTTP access logs for both paths.

### Phase 4 — SOCKS4/4a & SOCKS5 Engine
- Implement `socks/v4.rs` (SOCKS4 + SOCKS4a `CONNECT`).
- Implement `socks/v5.rs` (SOCKS5 no-auth `CONNECT` for IPv4, IPv6, Domain).
- Emit structured SOCKS connection logs.

### Phase 5 — MITM CA, TTL Cert Cache, HTTPS Interception & SOCKS Web Guard
- Implement `mitm/ca.rs` (PEM & PKCS#12 loader + startup validation).
- Implement `mitm/signer.rs` (`rcgen` leaf signer with `-5m` / `+TTL+5m` validity window).
- Implement `mitm/cache.rs` (`DashMap` TTL cache, single-flight dedup, lazy expiry check, background reaper task).
- Implement `http/mitm_tunnel.rs` (SNI-aware TLS termination, upstream TLS re-encryption, streaming HTTP/1.1 & HTTP/2 inspection, WebSocket upgrade passthrough).
- Implement `socks/web_guard.rs` (`--socks-block-web-in-mitm` plaintext HTTP + TLS ALPN/port-443 guard).

### Phase 6 — End-to-End Tests, Docs & Cross-Platform Release Polish
- Write automated integration tests in `tests/` using local mock HTTP/HTTPS/raw-TCP servers and ephemeral test CAs (verifying zero caching, cert TTL expiry & reaping, PEM & PKCS#12 loading, single-port mux, SOCKS4/4a/5, and `--socks-block-web-in-mitm`).
- Write `README.md` and `docs/MITM-CA-GUIDE.md`.
- Verify `cargo test`, `cargo clippy --all-targets`, `cargo fmt --check`, and cross-platform builds.
