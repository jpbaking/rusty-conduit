//! Shared TCP dial and relay utilities.
//!
//! Upstream connections resolve hostnames directly through the OS resolver (via Tokio's
//! `lookup_host`) on **every** dial: there is no DNS cache and no connection pooling.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::Instant;

use crate::filter::{access_denied_error, normalize_domain, AccessDenied, HostFilter};

/// Default timeout for establishing an upstream TCP connection.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn timed_out(host: &str, port: u16, timeout: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "connect to {host}:{port} timed out after {}",
            humantime::format_duration(timeout)
        ),
    )
}

/// Open a fresh TCP connection to `host:port` with `TCP_NODELAY` enabled.
///
/// `host` may be a DNS name or an IP literal (IPv6 literals with or without brackets). Name
/// resolution is performed by the OS on each call (zero DNS caching); every resolved address
/// is tried in order until one connects. The whole resolve + connect sequence is bounded by
/// `timeout`, after which an [`io::ErrorKind::TimedOut`] error is returned.
pub async fn connect_upstream(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    connect_upstream_filtered(host, port, timeout, None).await
}

/// Like [`connect_upstream`], additionally enforcing an optional destination [`HostFilter`].
///
/// When `filter` is `Some`, both domain rules and resolved-IP IP/CIDR rules are checked before
/// opening a TCP connection (so DNS-to-loopback / DNS-rebinding bypasses are rejected).
pub async fn connect_upstream_filtered(
    host: &str,
    port: u16,
    timeout: Duration,
    filter: Option<&HostFilter>,
) -> io::Result<TcpStream> {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    let Some(filter) = filter else {
        let stream = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
            .await
            .map_err(|_| timed_out(host, port, timeout))??;
        stream.set_nodelay(true)?;
        return Ok(stream);
    };

    let deadline = Instant::now() + timeout;
    if let Ok(ip) = host.parse::<IpAddr>() {
        filter.check_ip(ip).map_err(access_denied_error)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let stream = tokio::time::timeout(remaining, TcpStream::connect(SocketAddr::new(ip, port)))
            .await
            .map_err(|_| timed_out(host, port, timeout))??;
        stream.set_nodelay(true)?;
        return Ok(stream);
    }

    let domain = normalize_domain(host);
    let domain_matched = filter.matches_domain(&domain);
    match filter {
        HostFilter::Block(_) if domain_matched => {
            return Err(access_denied_error(AccessDenied(format!(
                "destination {host} is blocklisted"
            ))));
        }
        HostFilter::Allow(_) if !domain_matched && !filter.has_ip_rules() => {
            return Err(access_denied_error(AccessDenied(format!(
                "destination {host} is not in allowlist"
            ))));
        }
        _ => {}
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    let resolved: Vec<SocketAddr> = tokio::time::timeout(remaining, lookup_host((host, port)))
        .await
        .map_err(|_| timed_out(host, port, timeout))??
        .collect();
    if resolved.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("DNS returned no addresses for {host}"),
        ));
    }

    let candidates: Vec<SocketAddr> = match filter {
        HostFilter::Block(_) => {
            if let Some(blocked) = resolved.iter().find(|a| filter.matches_ip(a.ip())) {
                return Err(access_denied_error(AccessDenied(format!(
                    "destination {host} resolved to blocklisted IP {}",
                    blocked.ip()
                ))));
            }
            resolved
        }
        HostFilter::Allow(_) if domain_matched => resolved,
        HostFilter::Allow(_) => {
            let allowed: Vec<SocketAddr> = resolved
                .into_iter()
                .filter(|a| filter.matches_ip(a.ip()))
                .collect();
            if allowed.is_empty() {
                return Err(access_denied_error(AccessDenied(format!(
                    "destination {host} is not in allowlist"
                ))));
            }
            allowed
        }
    };

    let mut last_err = None;
    for addr in candidates {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(timed_out(host, port, timeout));
        }
        match tokio::time::timeout(remaining, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Ok(Err(e)) => last_err = Some(e),
            Err(_) => return Err(timed_out(host, port, timeout)),
        }
    }
    Err(last_err.unwrap_or_else(|| timed_out(host, port, timeout)))
}

/// Relay bytes in both directions until both sides reach EOF (or either side errors).
///
/// Returns `(bytes_in, bytes_out)` where `bytes_in` is the number of bytes read from
/// `client` and written to `upstream`, and `bytes_out` is the number of bytes read from
/// `upstream` and written to `client`. Data is streamed through fixed buffers and never
/// retained.
pub async fn relay_bidirectional<C, U>(client: &mut C, upstream: &mut U) -> io::Result<(u64, u64)>
where
    C: AsyncRead + AsyncWrite + Unpin + ?Sized,
    U: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    tokio::io::copy_bidirectional(client, upstream).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn relay_counts_bytes_in_both_directions() {
        // Upstream echo-ish server: reads the full request, replies with a fixed payload.
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            let (mut sock, _) = upstream_listener.accept().await.unwrap();
            let mut received = Vec::new();
            sock.read_to_end(&mut received).await.unwrap();
            sock.write_all(&[b'r'; 2500]).await.unwrap();
            sock.shutdown().await.unwrap();
            received
        });

        // Proxy front listener: accepts the client and relays to upstream.
        let front = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let front_addr = front.local_addr().unwrap();
        let relay_task = tokio::spawn(async move {
            let (mut client, _) = front.accept().await.unwrap();
            let mut upstream = connect_upstream(
                &upstream_addr.ip().to_string(),
                upstream_addr.port(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
            assert!(upstream.nodelay().unwrap());
            relay_bidirectional(&mut client, &mut upstream)
                .await
                .unwrap()
        });

        let mut client = TcpStream::connect(front_addr).await.unwrap();
        client.write_all(&[b'q'; 1234]).await.unwrap();
        client.shutdown().await.unwrap();
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();

        let (bytes_in, bytes_out) = relay_task.await.unwrap();
        let received = upstream_task.await.unwrap();
        assert_eq!(received, vec![b'q'; 1234]);
        assert_eq!(reply, vec![b'r'; 2500]);
        assert_eq!((bytes_in, bytes_out), (1234, 2500));
    }

    #[tokio::test]
    async fn connect_upstream_resolves_localhost_name() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
        let stream = connect_upstream("localhost", port, Duration::from_secs(5)).await;
        // `localhost` may resolve to ::1 first on some hosts; all addresses are tried.
        let stream = stream.unwrap();
        assert!(stream.nodelay().unwrap());
        accept.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn connect_upstream_reports_refused() {
        // Bind then drop to obtain a (very likely) closed port.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let err = connect_upstream("127.0.0.1", port, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn connect_upstream_accepts_bracketed_ipv6_literal() {
        let Ok(listener) = TcpListener::bind("[::1]:0").await else {
            return; // IPv6 loopback unavailable on this host.
        };
        let port = listener.local_addr().unwrap().port();
        let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
        connect_upstream("[::1]", port, Duration::from_secs(5))
            .await
            .unwrap();
        accept.await.unwrap().unwrap();
    }
}
