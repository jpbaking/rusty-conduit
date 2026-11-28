# Advanced User Guide & Configuration Reference

This guide covers full configuration precedence, TOML files, HTTPS MITM inspection, SOCKS web-traffic blocking, structured logging, custom proxy error pages, and operational hardening.

- New to `rusty-conduit`? Start with the **[Basic User Guide](USER-GUIDE-BASIC.md)**.
- For RAM/CPU budgets, `ulimit -n` sizing, and cap mechanics, see **[Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md)**.
- For creating and trusting a private CA, see the **[MITM CA Guide](MITM-CA-GUIDE.md)**.
- For containers and Compose, see the **[Docker Guide](DOCKER-GUIDE.md)**.

---

## 1. Configuration Precedence & Reference

Settings are merged in strict order (higher overrides lower):

1. **CLI flags**
2. **Environment variables** (`RUSTY_CONDUIT_*`)
3. **TOML config file** (`--config <PATH>`)
4. **Built-in defaults**

### Complete Reference

| CLI Flag | Environment Variable | TOML Key | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `--config <PATH>` | `RUSTY_CONDUIT_CONFIG` | — | *(none)* | Optional TOML configuration file (unknown keys are rejected) |
| `--bind <IP>` | `RUSTY_CONDUIT_BIND` | `bind` | `127.0.0.1` | IPv4 or IPv6 address for all listeners |
| `--http-port <PORT>` | `RUSTY_CONDUIT_HTTP_PORT` | `http_port` | `3128` | HTTP/HTTPS proxy port (`0` disables) |
| `--socks-port <PORT>` | `RUSTY_CONDUIT_SOCKS_PORT` | `socks_port` | `1080` | SOCKS4/4a/5 proxy port (`0` disables) |
| `--max-connections <N>` | `RUSTY_CONDUIT_MAX_CONNECTIONS` | `max_connections` | `4096` | Max concurrent connections across all listeners (`0` = unlimited) |
| `--tcp-keepalive <DURATION>` | `RUSTY_CONDUIT_TCP_KEEPALIVE` | `tcp_keepalive` | `60s` | Idle time before TCP keepalive probes start on client and upstream sockets (`0` disables; `1s`–`32767s`) |
| `--tcp-keepalive-interval <DURATION>` | `RUSTY_CONDUIT_TCP_KEEPALIVE_INTERVAL` | `tcp_keepalive_interval` | `10s` | Time between unanswered keepalive probes (`1s`–`32767s`) |
| `--tcp-keepalive-retries <N>` | `RUSTY_CONDUIT_TCP_KEEPALIVE_RETRIES` | `tcp_keepalive_retries` | `5` | Unanswered probes before the OS closes the connection (`1`–`127`) |
| `--allow <RULE>` | `RUSTY_CONDUIT_ALLOW` | `allow` | *(none)* | Destination allowlist (IP, CIDR, FQDN, or `*.domain`); when combined with `--block`, serves as exceptions to the blocklist |
| `--block <RULE>` | `RUSTY_CONDUIT_BLOCK` | `block` | *(none)* | Destination blocklist (IP, CIDR, FQDN, or `*.domain`); combine with `--allow` to carve out exceptions |
| `--mitm-ca-cert <PATH>` | `RUSTY_CONDUIT_MITM_CA_CERT` | `mitm.ca_cert` | *(none)* | PEM CA certificate (enables MITM with `--mitm-ca-key`) |
| `--mitm-ca-key <PATH>` | `RUSTY_CONDUIT_MITM_CA_KEY` | `mitm.ca_key` | *(none)* | PEM CA private key (PKCS#8, PKCS#1 RSA, SEC1 EC, or encrypted PKCS#8) |
| `--mitm-ca-p12 <PATH>` | `RUSTY_CONDUIT_MITM_CA_P12` | `mitm.ca_p12` | *(none)* | PKCS#12 (`.p12`/`.pfx`) CA bundle (enables MITM) |
| `--mitm-ca-passphrase <STR>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE` | `mitm.ca_passphrase` | *(none)* | CA passphrase *(prefer env var or passphrase file)* |
| `--mitm-ca-passphrase-file <PATH>` | `RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE` | `mitm.ca_passphrase_file` | *(none)* | File containing CA passphrase (trailing `\n`/`\r\n` trimmed) |
| `--mitm-cert-ttl <DURATION>` | `RUSTY_CONDUIT_MITM_CERT_TTL` | `mitm.cert_ttl` | `1h` | In-memory MITM leaf cert TTL (`humantime`, min `10s`) |
| `--mitm-cert-cache-max-entries <N>` | `RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES` | `mitm.cert_cache_max_entries` | `10240` | Max cached MITM leaf certs (`0` = unlimited). The older TOML key `mitm.cert_cache_max` is still read when `cert_cache_max_entries` is absent, with a startup notice |
| `--mitm-insecure-host <RULE>` | `RUSTY_CONDUIT_MITM_INSECURE_HOST` | `mitm.insecure_hosts` | *(none)* | Under MITM, accept invalid/self-signed upstream TLS certs for matching destinations (IP, CIDR, FQDN, or `*.domain`; same syntax as `--allow`); repeatable or comma-separated; CLI/env replaces the TOML list |
| `--mitm-insecure` | `RUSTY_CONDUIT_MITM_INSECURE` | `mitm.insecure` | `false` | **Unsafe** blind trust: skip upstream TLS certificate verification for ALL MITM targets; enabled if set in CLI/env **or** TOML; takes precedence over `--mitm-insecure-host` |
| `--mitm-bypass-host <RULE>` | `RUSTY_CONDUIT_MITM_BYPASS_HOST` | `mitm.bypass_hosts` | *(none)* | Under MITM, do **not** intercept `CONNECT`s whose target host matches (IP, CIDR, FQDN, or `*.domain`; same syntax as `--allow`): they get an opaque tunnel instead ([Excluding hosts from MITM](#excluding-hosts-from-mitm)); repeatable or comma-separated; CLI/env replaces the TOML list |
| `--socks-block-web` | `RUSTY_CONDUIT_SOCKS_BLOCK_WEB` | `socks_block_web` | `false` | Reject HTTP/HTTPS traffic over SOCKS4/4a/5 (works with or without MITM). |
| `--error-page <PATH>` | `RUSTY_CONDUIT_ERROR_PAGE` | `error_page` | *(built-in page)* | HTML template for proxy-generated HTTP error pages (placeholders `{{status}}`, `{{reason}}`, `{{message}}`, `{{target}}`); upstream responses are never altered ([section 7](#7-custom-proxy-error-pages)) |
| `--log-level <LEVEL>` | `RUSTY_CONDUIT_LOG_LEVEL` | `logging.level` | `INFO` | `QUIET`, `FATAL`, `ERROR`, `INFO`, `DEBUG`, `VERBOSE` (case-insensitive) |
| `--log-format <FMT>` | `RUSTY_CONDUIT_LOG_FORMAT` | `logging.format` | `text` | Access/SOCKS log format: `text` (logfmt) or `json` (NDJSON) |
| `--http-access-log <PATH>` | `RUSTY_CONDUIT_HTTP_ACCESS_LOG` | `logging.http_access_log` | *(none)* | File path for HTTP/HTTPS access log (append mode) |
| `--socks-log <PATH>` | `RUSTY_CONDUIT_SOCKS_LOG` | `logging.socks_log` | *(none)* | File path for SOCKS connection log (append mode) |

### Example TOML File

```toml
# rusty-conduit.toml — every key is optional; unknown keys fail startup.
bind = "127.0.0.1"
http_port = 3128
socks_port = 1080
max_connections = 4096
tcp_keepalive = "60s"          # "0" disables
tcp_keepalive_interval = "10s"
tcp_keepalive_retries = 5
# Optional destination filter — combine `block` with `allow` to carve out exceptions:
# block = ["127.0.0.0/8", "::1", "10.0.0.0/8", "*.internal"]
# allow = ["example.com", "*.example.org", "203.0.113.0/24"]
socks_block_web = true
# Optional custom HTML template for proxy-generated error pages (default: built-in page):
# error_page = "/etc/rusty-conduit/error.html"

[mitm]
# Use EITHER a PEM pair...
ca_cert = "/etc/rusty-conduit/ca.pem"
ca_key  = "/etc/rusty-conduit/ca.enc.key"
# ...OR a PKCS#12 bundle (never both):
# ca_p12 = "/etc/rusty-conduit/ca.p12"
ca_passphrase_file = "/etc/rusty-conduit/ca.pass"
cert_ttl = "30m"
cert_cache_max_entries = 10240
# Skip upstream TLS verification only for these lab / self-signed origins:
# insecure_hosts = ["dev.internal", "*.lab.example", "10.20.0.0/16"]
# UNSAFE: skip upstream TLS verification for every destination:
# insecure = false
# Never intercept these (pinned apps, banking, health): opaque CONNECT tunnel instead:
# bypass_hosts = ["*.bank.example", "pinned.example", "203.0.113.0/24"]

[logging]
level = "INFO"
format = "json"
http_access_log = "/var/log/rusty-conduit/access.log"
socks_log = "/var/log/rusty-conduit/socks.log"
```

Override selectively at runtime:

```sh
rusty-conduit --config /etc/rusty-conduit/rusty-conduit.toml
RUSTY_CONDUIT_LOG_LEVEL=DEBUG rusty-conduit --config /etc/rusty-conduit/rusty-conduit.toml
rusty-conduit --config /etc/rusty-conduit/rusty-conduit.toml --http-port 8080
```

### Startup Validation & Notices

**Fatal startup errors (exit code `1`):**
- Both `http_port` and `socks_port` are `0`.
- Mixing `--mitm-ca-p12` with `--mitm-ca-cert` or `--mitm-ca-key`.
- Supplying only one of `--mitm-ca-cert` or `--mitm-ca-key`.
- Supplying both `--mitm-ca-passphrase` and `--mitm-ca-passphrase-file`.
- `--mitm-cert-ttl` shorter than `10s`.
- An invalid `--mitm-insecure-host` / `mitm.insecure_hosts` rule.
- An invalid `--mitm-bypass-host` / `mitm.bypass_hosts` rule.
- `--tcp-keepalive` not `0` and outside `1s`–`32767s`; or, while keepalive is enabled, `--tcp-keepalive-interval` outside `1s`–`32767s` or `--tcp-keepalive-retries` outside `1`–`127`.
- Unreadable, expired, not-yet-valid, or mismatched CA cert/key, wrong PKCS#12 passphrase, or legacy `Proc-Type: 4,ENCRYPTED` PEM key.
- Port bind failure or unopenable log file path.
- Unreadable `--error-page` / `error_page` template file (the error names the path).

**`INFO` startup notices:**
- Binding to a non-loopback IP (reminds that there is no proxy authentication).
- Passing `--mitm-ca-passphrase` as a CLI argument (recommends env var or passphrase file).

**`WARN:` startup lines (logged at `INFO`, only while MITM is enabled):**
- `--mitm-insecure` / `mitm.insecure`: upstream HTTPS certificates are not verified for any destination.
- `--mitm-insecure-host` / `mitm.insecure_hosts`: names the count and list of rules whose upstream verification is skipped.

### Checking a configuration

Use the `validate` subcommand to load, merge, and validate CLI flags, environment variables, and `--config` without binding any listener. Other flags go before the subcommand:

```sh
rusty-conduit validate --config proxy.toml
rusty-conduit --socks-port 0 validate --config proxy.toml
```

- On success, it prints `config OK: <path|none>` (followed by any startup `NOTICE:` lines) to `stdout` and exits `0`.
- On failure, it prints `config invalid: <error>` to `stderr` and exits `1`.
- It also checks the files the configuration names, as startup would: it loads the MITM CA (files, passphrase, validity, and that the key matches the certificate), checks that each log file can be opened, without creating it, and reads the `--error-page` template. It prints the open-file `NOTICE` startup would log, too.
- `--settings-only` skips those file checks, for checking a config away from the host it runs on, for example in CI where the CA exists only on the server: `rusty-conduit validate --settings-only --config proxy.toml`.

**Exit codes:** `0` for a clean exit, `1` for a fatal startup or runtime error, and `2` for a command-line usage error.

---

## 2. HTTPS MITM Inspection

`rusty-conduit` **never auto-generates a CA**. See [docs/MITM-CA-GUIDE.md](MITM-CA-GUIDE.md) to generate and trust your own PEM or PKCS#12 CA.

```sh
# PEM cert + encrypted PKCS#8 key
rusty-conduit \
  --mitm-ca-cert ca.pem \
  --mitm-ca-key ca.enc.key \
  --mitm-ca-passphrase-file ca.pass \
  --http-access-log access.log

# PKCS#12 bundle with passphrase via environment variable
RUSTY_CONDUIT_MITM_CA_PASSPHRASE='...' rusty-conduit \
  --mitm-ca-p12 ca.p12 \
  --http-access-log access.log
```

### How It Works
- Applies strictly to HTTP-proxy `CONNECT` tunnels (SOCKS remains a pure L4 TCP tunnel).
- Inspects TLS `ClientHello` SNI (falling back to the `CONNECT` host if SNI is absent).
- Signs a per-host ECDSA P-256 leaf certificate with a random 128-bit serial and single DNS/IP SAN:
  - `not_before = now - 5m` (clock-skew tolerance)
  - `not_after  = now + TTL + 5m` (expiry grace buffer so certs handed out near TTL end never expire mid-handshake)
- Concurrent cache misses for the same host are deduplicated (single-flight), so 50 simultaneous handshakes sign only **1** leaf.
- Verifies upstream (origin) TLS certificates against the OS root certificate store — invalid origin certs are rejected and logged (`status=502`). The startup `config:` line shows the mode as `mitm_upstream_tls=strict|insecure_hosts(N)|insecure_all`:
  - `--mitm-insecure-host <RULE>` accepts invalid or self-signed certs only for matching targets (exact domain, `*.domain`, IP, or CIDR); every other target is still verified.
  - `--mitm-insecure` accepts any upstream cert for every target (blind trust). Use it only on isolated test networks: anyone on the path to the origin can impersonate it.
- Supports HTTP/1.1, HTTP/2 (mirroring upstream ALPN), and `101 Switching Protocols` upgrades (e.g., WebSocket) with zero response caching.
- Logs every decrypted request with `proto=https mitm=true` and the full `https://` URL.

### Excluding hosts from MITM

`--mitm-bypass-host <RULE>` (env `RUSTY_CONDUIT_MITM_BYPASS_HOST`, TOML `[mitm] bypass_hosts = [...]`) lists destinations that are never intercepted, for example certificate-pinned apps or banking and health sites:

```sh
rusty-conduit --mitm-ca-p12 ca.p12 \
  --mitm-bypass-host '*.bank.example,pinned.example' \
  --mitm-bypass-host 203.0.113.0/24
```

- A matching `CONNECT` gets the same opaque tunnel as without MITM: no leaf certificate is signed, the client sees the origin's real certificate, and the access log records `mitm=false`.
- Rules use the `--allow` syntax: exact domain, `*.domain`, IP, or CIDR. They match the `CONNECT` target host as the client sent it, with no DNS lookup, so an IP or CIDR rule matches only clients that `CONNECT` to an IP literal.
- The bypass list does not relax anything: `--allow` / `--block` still apply to bypassed targets, and `--mitm-insecure-host` / `--mitm-insecure` concern only intercepted targets.
- Repeat the flag or separate rules with commas; a CLI or env list replaces the TOML list. An invalid rule is a `FATAL` startup error. The startup `config:` line shows the rule count as `mitm_bypass_hosts=N`.

---

## 3. Blocking Web Traffic over SOCKS

Clients could otherwise bypass HTTPS logging by sending web traffic over SOCKS. Enable `--socks-block-web` to prevent that (it also works without MITM):

```sh
rusty-conduit \
  --mitm-ca-cert ca.pem \
  --mitm-ca-key ca.key \
  --socks-block-web \
  --http-access-log access.log \
  --socks-log socks.log
```

**Inspection mechanics:**
- Upstream → client data always flows immediately, so server-speaks-first protocols (**SSH, SMTP, Postgres, MySQL, etc.**) are never delayed or broken.
- Before forwarding the **first** client → upstream payload chunk, `rusty-conduit` checks for:
  1. **Plaintext HTTP methods:** `GET `, `POST `, `HEAD `, `PUT `, `DELETE `, `OPTIONS `, `PATCH `, `CONNECT `, `TRACE `.
  2. **HTTPS (`ClientHello`):** TLS handshake (`0x16 0x03`) advertising ALPN `http/1.1` or `h2` on any port, or targeting port `443` without advertising a non-HTTP ALPN.
- Matching sessions are closed immediately without forwarding the chunk and recorded in the SOCKS log with `status=blocked_web_in_mitm`.

---

## 4. Logging & Observability

### Stream Separation & Levels

Diagnostic logs always write to **`stderr`**. Access and SOCKS logs write to their configured files, and at `DEBUG` / `VERBOSE` are **also** copied to **`stdout`** so you can pipe them independently.

Log files are written through a buffer that is flushed whenever no more records are queued, so records reach the file within moments even on a quiet proxy, while bursts are still written in large blocks. Records on `stdout` are written immediately.

| Level | `stderr` (Diagnostics) | Access & SOCKS Logs |
| :--- | :--- | :--- |
| `QUIET` | None | Files only (if configured) |
| `FATAL` | Unrecoverable startup errors | Files only |
| `ERROR` | `FATAL` + runtime / connection errors | Files only |
| `INFO` *(default)* | `ERROR` + startup banner, config summary, notices, shutdown | Files only |
| `DEBUG` | `INFO` + per-connection lifecycle, rejected handshakes, cap drops | **Files AND `stdout`** |
| `VERBOSE` | `DEBUG` + cert reaper & cache eviction stats | **Files AND `stdout`** |

### Log Formats (`text` vs. `json`)

**`text` (default, logfmt):**

```text
ts=2026-11-22T13:30:01.123Z kind=http client=127.0.0.1:51234 proto=https mitm=true method=GET target=https://example.com/api/v1 status=200 bytes_in=142 bytes_out=4096 duration_ms=84
ts=2026-11-22T13:30:02.456Z kind=socks client=127.0.0.1:51240 proto=socks5 cmd=CONNECT target=github.com:22 status=ok bytes_in=1280 bytes_out=8192 duration_ms=1520
```

**`json` (`--log-format json`, NDJSON):**

```json
{"ts":"2026-11-22T13:30:01.123Z","kind":"http","client":"127.0.0.1:51234","proto":"https","mitm":true,"method":"GET","target":"https://example.com/api/v1","status":200,"bytes_in":142,"bytes_out":4096,"duration_ms":84}
```

Example live filtering with `jq`:

```sh
rusty-conduit --log-level DEBUG --log-format json 2>/dev/null \
  | jq -r 'select(.kind=="http" and .status>=400) | "\(.ts) \(.status) \(.method) \(.target)"'
```

### Field Reference

- **HTTP (`kind=http`):** `ts`, `client`, `proto` (`http`|`https`), `mitm` (`true`|`false`), `method`, `target`, `status`, `bytes_in`, `bytes_out`, `duration_ms`.
- **SOCKS (`kind=socks`):** `ts`, `client`, `proto` (`socks4`|`socks4a`|`socks5`), `cmd`, `target`, `status` (`ok`, `not_allowed`, `blocked_web_in_mitm`, `command_not_supported`, `address_type_not_supported`, `no_acceptable_auth`, `connection_refused`, `host_unreachable`, `network_unreachable`, `general_failure`), `bytes_in`, `bytes_out`, `duration_ms`.

---

## 5. Connection Lifetime & TCP Keepalive

Once a tunnel, SOCKS relay, or HTTP connection is established, rusty-conduit applies **no idle or total-lifetime timeout**: a connection stays open as long as both peers keep it open, however long it sits silent. The only time limits are setup-phase ones (protocol sniff `5s`, SOCKS handshake `10s`, upstream connect `10s`, MITM TLS handshake `10s`) and the `5s` drain on shutdown.

Silent connections can still be dropped **outside** the proxy: NAT gateways, firewalls, VPN concentrators (e.g. OpenVPN servers), Docker/Kubernetes conntrack, and cloud load balancers expire idle flows, often without notifying either end. Long-polling clients, such as AI agents waiting on a model response, are the typical victims.

To keep that state alive, rusty-conduit enables OS-level **TCP keepalive** on every accepted client socket and every upstream socket (HTTP forward, `CONNECT`, MITM, SOCKS4/4a/5). By default, the first probe is sent after `60s` idle, then every `10s`, and the connection is closed after `5` unanswered probes (~110s after the peer vanishes).

- Probes are empty TCP segments handled by the kernel. They are invisible to HTTP, TLS, SOCKS, and any tunneled protocol (SSH, database protocols, etc.) and add ~60 bytes per idle interval.
- Probes keep each hop (client ↔ proxy, proxy ↔ destination) alive. They are not forwarded end to end, and they cannot save a connection whose path is fully broken, such as an OpenVPN tunnel restart.
- For SSH through SOCKS, also set `ServerAliveInterval` (e.g. `30`) in `~/.ssh/config`. It works end to end inside the encrypted session.
- Set `--tcp-keepalive 0` to disable. On platforms without per-socket interval/retry control, only the idle time is applied.

## 6. Protocol Matrix

| Protocol | Supported | Not supported yet (see [TODO](TODO.md)) |
| :--- | :--- | :--- |
| HTTP proxy | Absolute-form `http://` forwarding (RFC 9110 hop-by-hop stripping), `CONNECT` tunnels, optional HTTPS MITM (HTTP/1.1, HTTP/2, `101` upgrades) | `Proxy-Authorization` |
| SOCKS4 / 4a | `CONNECT` (IPv4 + proxy-resolved domains); user ID logged as metadata | `BIND` (rejected `0x5B`) |
| SOCKS5 | No-auth `CONNECT` (IPv4, IPv6, Domain) | Username/password auth; `BIND` & `UDP ASSOCIATE` (rejected `0x07`) |

For tuning `--max-connections`, `--mitm-cert-cache-max-entries`, and `ulimit -n`, see **[Performance, Footprint & Sizing](PERFORMANCE-AND-SIZING.md)**.

---

## 7. Custom Proxy Error Pages

When `rusty-conduit` itself answers an HTTP request with an error — a malformed request (`400`), a destination denied by `--allow` / `--block` (`403`), an upstream it cannot reach or verify (`502`), a timeout (`504`), and so on — it sends an HTML page. By default this is a small, unbranded built-in page. Replace it with your own template:

```sh
rusty-conduit --error-page /etc/rusty-conduit/error.html
# or: RUSTY_CONDUIT_ERROR_PAGE=/etc/rusty-conduit/error.html
# or, in the TOML file (top level): error_page = "/etc/rusty-conduit/error.html"
```

The template is a UTF-8 file read once at startup. These placeholders are replaced in every page, and every value is HTML-escaped:

| Placeholder | Value |
| :--- | :--- |
| `{{status}}` | Numeric status code, for example `502` |
| `{{reason}}` | Standard reason phrase, for example `Bad Gateway` |
| `{{message}}` | The proxy's explanation of the error |
| `{{target}}` | The requested URL or `host:port` (never includes userinfo); empty when unknown |

```html
<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{{status}} {{reason}}</title></head>
<body><h1>{{status}} {{reason}}</h1><p>{{message}}</p><p><code>{{target}}</code></p></body></html>
```

- The template applies to every proxy-generated HTTP error: plain forwarding, `CONNECT`, and decrypted MITM requests. Pages are sent as `text/html; charset=utf-8` with `Cache-Control: no-store`.
- **Responses from an upstream origin are never altered**, error statuses included: a `404` or `500` from the origin reaches the client exactly as sent.
- SOCKS has no HTTP error responses, so the template does not apply to it.
- The startup `config:` line shows `error_page=<path>` or `error_page=builtin`. An unreadable file is a `FATAL` startup error; `validate` reports it too, unless `--settings-only`.
