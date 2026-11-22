//! Shared TCP dial and relay utilities.
//!
//! Upstream connections resolve hostnames directly through the OS resolver (via Tokio's
//! `lookup_host`) on **every** dial: there is no DNS cache and no connection pooling.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// Default timeout for establishing an upstream TCP connection.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Open a fresh TCP connection to `host:port` with `TCP_NODELAY` enabled.
///
/// `host` may be a DNS name or an IP literal (IPv6 literals without brackets). Name
/// resolution is performed by the OS on each call (zero DNS caching); every resolved address
/// is tried in order until one connects. The whole resolve + connect sequence is bounded by
/// `timeout`, after which an [`io::ErrorKind::TimedOut`] error is returned.
pub async fn connect_upstream(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let stream = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "connect to {host}:{port} timed out after {}",
                    humantime::format_duration(timeout)
                ),
            )
        })??;
    stream.set_nodelay(true)?;
    Ok(stream)
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
