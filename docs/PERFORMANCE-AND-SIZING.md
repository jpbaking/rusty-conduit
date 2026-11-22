# Performance, Footprint & Sizing Guide

This guide documents `rusty-conduit`'s memory, CPU, and file-descriptor characteristics, along with sizing rules of thumb and how its resource caps behave under load.

- Back to [README](../README.md) · [Basic User Guide](USER-GUIDE-BASIC.md) · [Advanced User Guide](USER-GUIDE-ADVANCED.md)

---

## 1. Zero-Caching Architecture

`rusty-conduit` is a streaming pass-through proxy. Keeping memory predictable starts with what it **never** retains:

| Component | Cached / Buffered? |
| :--- | :--- |
| HTTP responses (plain or decrypted MITM) | **Never.** Every request hits the origin regardless of `Cache-Control` / `Expires`. |
| DNS lookups | **Never.** Every upstream dial resolves via the OS resolver. |
| Upstream TCP/TLS connections | **Never pooled across clients.** |
| Request / response bodies | **Never buffered in full.** Streamed frame by frame. |
| MITM leaf certificates | **Yes — the only cache.** Bounded by `--mitm-cert-ttl` (default `1h`) and `--mitm-cert-cache-max-entries` (default `10000`). |

---

## 2. Memory & CPU Footprint

The **measured** figures come from a release build on 64-bit Linux using idle `CONNECT` tunnels over loopback after 1 KiB of traffic each (process RSS; kernel socket buffers excluded). **Estimated** figures were not benchmarked directly — validate under your own traffic profile.

| Workload / Component | RAM | CPU | Basis |
| :--- | :--- | :--- | :--- |
| Idle process | ~6 MB RSS | ~0% (event-driven) | Measured (`5.8 MB`) |
| Opaque tunnel (`CONNECT` or SOCKS) | ~25 KB / connection | Low (streaming byte copy; multi-Gbps per core) | Measured for `CONNECT` (`23.7 KB`); SOCKS estimated |
| Plain HTTP request | ~25–50 KB while active | Low (HTTP/1.1 header parsing + streaming) | Estimated |
| HTTPS MITM tunnel | ~50–150 KB / tunnel | Moderate–High (**2× TLS** via `ring` assembly AES-GCM/ChaCha20 + HTTP/1.1 or HTTP/2 framing) | Estimated |
| Cached MITM leaf certificate | ~3–5 KB / host (~30–50 MB at default `10000` cap) | ~0.2–0.5 ms per cache miss (ECDSA P-256 via `rcgen`, offloaded to blocking pool with single-flight dedup) | Estimated |

- The Tokio async runtime spawns one worker thread per CPU core.
- Throughput is typically network-bound for opaque tunnels and plain HTTP, and CPU-bound (double TLS) in MITM mode.
- The background MITM cert-cache reaper wakes once every `clamp(ttl / 4, 5s, 60s)` (once per minute at the default `1h` TTL) with negligible CPU overhead.

---

## 3. File Descriptors (`ulimit -n`)

Every active proxied connection uses **2 file descriptors** (1 client socket + 1 upstream socket). A MITM tunnel can briefly use extra descriptors when re-dialing an upstream connection.

Size your process file-descriptor limit using:

```text
ulimit -n  >=  (2 × max_connections) + 64
```

The default `--max-connections 4096` requires at least **`8256`** file descriptors. Many Linux shells default to `1024` (~480 concurrent connections), so either raise `ulimit -n` or lower `--max-connections`:

```sh
# Option A: raise FD limit to match default cap
ulimit -n 8256 && rusty-conduit

# Option B: stay comfortably inside a 1024 FD limit
rusty-conduit --max-connections 480
```

Under `systemd`, set `LimitNOFILE=8256` (or higher) in your service unit. If the OS runs out of file descriptors before `--max-connections` is reached, `accept` fails, `rusty-conduit` logs an `ERROR`, and the listener retries after `100 ms`.

---

## 4. Concurrency & Sizing Rules of Thumb

Assuming a typical active browser holds roughly **10 concurrent TCP connections** (varies widely with HTTP/2 multiplexing and workload):

| Deployment Profile | `--max-connections` | Recommended `ulimit -n` | Approx. RAM (Opaque) | Approx. RAM (MITM) | Approx. Active Users |
| :--- | :--- | :--- | :--- | :--- | :--- |
| Personal / single device | `256` | `1024` | ~15 MB | up to ~60 MB | 1–25 |
| Home / small team | `1024` | `2112` | ~35 MB | up to ~200 MB | 25–100 |
| Default | `4096` | `8256` | ~110 MB | up to ~700 MB | 100–400 |
| High concurrency (non-MITM) | `16384` | `32832` | ~420 MB | — | 1,000–2,500+ |

---

## 5. How the Resource Caps Behave

### `--max-connections` (default `4096`, `0` = unlimited)
- Shared across all active listeners (HTTP, SOCKS, or multiplexed).
- Uses a non-blocking semaphore (`try_acquire_owned()`). When the cap is reached, new incoming connections are **accepted and closed immediately** (no reply, no queue, no slowloris buildup) and logged at `DEBUG`.
- Existing connections are unaffected, and a slot is freed immediately when any connection or tunnel ends.

### `--mitm-cert-cache-max-entries` (default `10000`, `0` = unlimited)
- Strictly bounds the number of in-memory MITM leaf certificates.
- When a new hostname is signed at the cap:
  1. Already-expired entries are reaped first so live certificates are not evicted unnecessarily.
  2. If the cache is still at the cap, the **oldest** entries (closest to expiry) are evicted and logged at `VERBOSE`.
- Cache hits are lock-free and unaffected; an evicted host is simply re-signed on its next connection.

> **Security note:** Setting either cap to `0` disables the upper bound. Avoid `0` on untrusted networks where a client could exhaust file descriptors or flood random SNI hostnames.
