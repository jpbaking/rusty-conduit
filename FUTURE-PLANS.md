# Rusty Conduit — Future Plans (Post-v1)

This document tracks features intentionally deferred from v1 (see [docs/MASTER-PLAN.md](docs/MASTER-PLAN.md)) so the initial release stays focused, clean, and reliable.

---

## 1. Proxy Authentication
- **HTTP/HTTPS:** `Proxy-Authorization: Basic` (RFC 7235 / 9110) with `407 Proxy Authentication Required` challenges.
- **SOCKS5:** Username/Password authentication (RFC 1929, auth method `0x02`).
- **SOCKS4 Compatibility:** Automatically reject/disable SOCKS4 when authentication is enabled (since SOCKS4 lacks password auth).
- **Credential Storage:** Support multiple users via TOML config or htpasswd-style hashed credential files (Argon2/bcrypt) rather than plaintext CLI flags.

## 2. SOCKS5 `UDP ASSOCIATE` (and Optional `BIND`)
- Full RFC 1928 `UDP ASSOCIATE` (`0x03`) support for DNS-over-UDP, QUIC/HTTP3, and UDP-based protocols.
- Ephemeral UDP relay socket management, client-IP restriction, idle timeouts, and UDP session logging.
- Optional `BIND` (`0x02`) command support if needed for legacy active-mode protocols.

## 3. Optional TLS-over-SOCKS MITM Inspection
- Opt-in flag (e.g. `--mitm-socks-tls`) to sniff TLS `ClientHello` on SOCKS connections, inspect decrypted HTTP/1.1 or HTTP/2 traffic when HTTP ALPN is detected, and transparently pass through non-HTTP TLS protocols.

## 4. Fine-Grained MITM Bypass / Allowlist / Denylist
- Domain/wildcard/CIDR rules (e.g. `--mitm-bypass "*.apple.com,*.bank.example"`) to transparently tunnel certificate-pinned apps or sensitive domains without MITM interception while keeping MITM active for everything else.

## 5. Destination ACLs & Upstream Proxy Chaining
- Optional SSRF / private-network guardrails (block connections to loopback, link-local, RFC 1918, or cloud metadata IPs like `169.254.169.254` when exposing the proxy on a LAN).
- Upstream proxy chaining (forward traffic through another HTTP or SOCKS5 proxy).

## 6. Operational & Observability Enhancements
- **Log Rotation Support:** Reopen `--http-access-log` and `--socks-log` on `SIGHUP` (Unix) for seamless `logrotate` integration, plus optional built-in size/time rotation.
- **Metrics Endpoint:** Optional lightweight Prometheus `/metrics` listener (active connections, bytes transferred, MITM cert cache hit/miss/eviction counts).
- **Config Reload:** Hot-reload non-listener settings (log level, bypass lists) without dropping active tunnels.

## 7. OS Service Packaging & Distribution
- Prebuilt release binaries via GitHub Actions for Linux (glibc + musl), Windows (MSVC), and macOS (Intel + Apple Silicon).
- Sample service definitions: `systemd` unit (Linux), `launchd` plist (macOS), and Windows Service wrapper support.
