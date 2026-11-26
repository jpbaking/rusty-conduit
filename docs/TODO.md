# TODO

Features not yet in `rusty-conduit`. The [Advanced User Guide](USER-GUIDE-ADVANCED.md#6-protocol-matrix) lists what each protocol supports today.

## 1. Proxy authentication
- **HTTP/HTTPS:** `Proxy-Authorization: Basic` (RFC 7235 / 9110) with `407 Proxy Authentication Required` challenges.
- **SOCKS5:** Username/password authentication (RFC 1929, auth method `0x02`).
- **SOCKS4:** Reject SOCKS4 when authentication is enabled, since it has no password authentication.
- **Credential storage:** Several users in the TOML config or in an htpasswd-style file with hashed passwords (Argon2 or bcrypt), rather than plaintext CLI flags.

## 2. SOCKS5 `UDP ASSOCIATE` and `BIND`
- RFC 1928 `UDP ASSOCIATE` (`0x03`) for DNS over UDP, QUIC/HTTP3, and other UDP protocols, with relay socket management, client-IP restriction, idle timeouts, and UDP session logging.
- `BIND` (`0x02`) for legacy active-mode protocols, if needed.

## 3. TLS-over-SOCKS MITM inspection
- An opt-in flag (for example `--mitm-socks-tls`) to sniff the TLS `ClientHello` on SOCKS connections, inspect decrypted HTTP/1.1 or HTTP/2 traffic when HTTP ALPN is detected, and pass other TLS protocols through unchanged.

## 4. MITM bypass list
- Domain, wildcard, and CIDR rules (for example `--mitm-bypass "*.apple.com,*.bank.example"`) that tunnel certificate-pinned apps or sensitive domains without interception, while MITM stays on for everything else.

## 5. Private-network guardrails and upstream chaining
- A built-in preset that blocks loopback, link-local, RFC 1918, and cloud metadata addresses (such as `169.254.169.254`) when the proxy is exposed on a LAN. Today `--block` can list these ranges by hand.
- Upstream proxy chaining: forward traffic through another HTTP or SOCKS5 proxy. The shared engine already supports it, and `rusty-conduit-switch` exposes it.

## 6. Operations and observability
- **Log rotation:** Reopen `--http-access-log` and `--socks-log` on `SIGHUP` (Unix) for `logrotate`, and optionally rotate by size or time.
- **Metrics:** An optional Prometheus `/metrics` listener (active connections, bytes transferred, MITM cert-cache hits, misses, and evictions).
- **Config reload:** Hot-reload settings that do not affect listeners (log level, filters) without dropping active tunnels.

## 7. Packaging
- Static `musl` Linux builds alongside the current glibc builds.
- Sample service definitions: a `systemd` unit (Linux), a `launchd` plist (macOS), and a Windows service wrapper.
