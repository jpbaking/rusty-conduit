//! End-to-end tests for listener planning: single-port first-byte multiplexing
//! (`http_port == socks_port`) serving plain HTTP, HTTPS `CONNECT`, SOCKS4, and SOCKS5 on
//! one port, and port `0` disabling a listener.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use rusty_conduit::build_engines;
use rusty_conduit::config::{CliInput, Config, ConfigError};
use rusty_conduit::logging::{AccessLogConfig, AccessLogger, LogFormat, LogLevel, Logger};
use rusty_conduit::server::{ListenPlan, ListenerRole, ProtocolHandlers, Server, ServerError};
use rusty_conduit::EngineHandlers;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn quiet_logger() -> Logger {
    Logger::with_writer(LogLevel::Quiet, Box::new(std::io::sink()))
}

fn temp_dir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rc-it-mux-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

async fn wait_lines(access: &AccessLogger, path: &Path, n: usize) -> Vec<String> {
    for _ in 0..250 {
        access.flush().await.unwrap();
        let lines = read_lines(path);
        if lines.len() >= n {
            return lines;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "expected {n} lines in {}, got {:?}",
        path.display(),
        read_lines(path)
    );
}

/// An OS-assigned port that is free right now (for configs that need a concrete port).
async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Build a server for `cfg` (ports taken from the config, as `rusty-conduit` does).
async fn server_for(cfg: &Config, access: &AccessLogger) -> Server {
    let logger = quiet_logger();
    let (http, socks) = build_engines(cfg, None, access, &logger);
    let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
    Server::from_options(&cfg.server_options(), handlers, logger)
        .await
        .unwrap()
}

/// HTTP/1.1 origin returning `origin:<path>` and closing.
async fn spawn_origin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut reader = BufReader::new(&mut sock);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).await.unwrap();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = request_line.split(' ').nth(1).unwrap_or("?").to_string();
                let body = format!("origin:{path}");
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
                let _ = sock.shutdown().await;
            });
        }
    });
    addr
}

async fn spawn_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

async fn roundtrip(stream: &mut TcpStream, payload: &[u8]) -> Vec<u8> {
    stream.write_all(payload).await.unwrap();
    let mut back = vec![0u8; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut back))
        .await
        .expect("echo timed out")
        .unwrap();
    back
}

async fn http_get(proxy: SocketAddr, origin: SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(
        format!(
            "GET http://{origin}{path} HTTP/1.1\r\nHost: {origin}\r\nConnection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut out))
        .await
        .expect("HTTP response timed out")
        .unwrap();
    out
}

async fn https_connect(proxy: SocketAddr, target: SocketAddr) -> TcpStream {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).await.unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(
        head.starts_with("HTTP/1.1 200 Connection Established\r\n"),
        "{head}"
    );
    s
}

async fn socks4_connect(proxy: SocketAddr, target: SocketAddr) -> TcpStream {
    let SocketAddr::V4(t) = target else {
        panic!("IPv4 target expected")
    };
    let mut req = vec![4, 1];
    req.extend_from_slice(&t.port().to_be_bytes());
    req.extend_from_slice(&t.ip().octets());
    req.push(0);
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 8];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep[1], 0x5A);
    s
}

async fn socks5_connect(proxy: SocketAddr, target: SocketAddr) -> TcpStream {
    let SocketAddr::V4(t) = target else {
        panic!("IPv4 target expected")
    };
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await.unwrap();
    assert_eq!(sel, [5, 0]);
    let mut req = vec![5, 1, 0, 1];
    req.extend_from_slice(&t.ip().octets());
    req.extend_from_slice(&t.port().to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep[1], 0x00);
    s
}

/// Whether the peer closes the connection without answering `probe`.
async fn closed_without_reply(addr: SocketAddr, probe: &[u8]) -> bool {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let _ = s.write_all(probe).await;
    let mut out = Vec::new();
    match tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await {
        Ok(_) => out.is_empty(),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_port_serves_http_https_socks4_and_socks5() {
    let origin = spawn_origin().await;
    let echo = spawn_echo().await;
    let dir = temp_dir("all");
    let (http_log, socks_log) = (dir.join("http.log"), dir.join("socks.log"));
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: port,
        http_access_log: Some(http_log.clone()),
        socks_log: Some(socks_log.clone()),
        ..Config::default()
    };
    cfg.validate().unwrap();
    assert!(cfg.is_multiplexed());
    assert_eq!(
        ListenPlan::from_options(&cfg.server_options()),
        ListenPlan::Multiplexed(SocketAddr::from(([127, 0, 0, 1], port)))
    );

    let access = AccessLogger::start(
        AccessLogConfig::new(
            LogLevel::Info,
            cfg.log_format,
            cfg.http_access_log.clone(),
            cfg.socks_log.clone(),
        ),
        quiet_logger(),
    )
    .await
    .unwrap();
    let server = server_for(&cfg, &access).await;
    assert!(server.is_multiplexed());
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    assert_eq!(server.listeners(), vec![(ListenerRole::Multiplexed, addr)]);
    assert_eq!(server.local_http_addr(), Some(addr));
    assert_eq!(server.local_socks_addr(), Some(addr));
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    // Plain HTTP.
    let resp = http_get(addr, origin, "/mux").await;
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "{resp}");
    assert!(resp.ends_with("origin:/mux"), "{resp}");

    // HTTPS CONNECT tunnel.
    let mut s = https_connect(addr, echo).await;
    assert_eq!(roundtrip(&mut s, b"tunnelled").await, b"tunnelled");
    drop(s);

    // SOCKS4 and SOCKS5 on the very same port.
    let mut s = socks4_connect(addr, echo).await;
    assert_eq!(roundtrip(&mut s, b"via socks4").await, b"via socks4");
    drop(s);
    let mut s = socks5_connect(addr, echo).await;
    assert_eq!(roundtrip(&mut s, b"via socks5").await, b"via socks5");
    drop(s);

    // Interleaved concurrent clients of every protocol.
    let mut tasks = Vec::new();
    for i in 0..4u8 {
        tasks.push(tokio::spawn(async move {
            let mut s = match i {
                0 => https_connect(addr, echo).await,
                1 => socks4_connect(addr, echo).await,
                2 => socks5_connect(addr, echo).await,
                _ => {
                    let r = http_get(addr, origin, "/concurrent").await;
                    assert!(r.ends_with("origin:/concurrent"), "{r}");
                    return;
                }
            };
            let msg = [b'c', i];
            assert_eq!(roundtrip(&mut s, &msg).await, msg);
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }

    let http = wait_lines(&access, &http_log, 4).await;
    assert!(
        http.iter()
            .any(|l| l.contains("proto=http mitm=false method=GET")),
        "{http:?}"
    );
    assert!(
        http.iter()
            .any(|l| l.contains("proto=https mitm=false method=CONNECT")),
        "{http:?}"
    );
    let socks = wait_lines(&access, &socks_log, 4).await;
    assert!(
        socks.iter().any(|l| l.contains("proto=socks4 cmd=CONNECT")),
        "{socks:?}"
    );
    assert!(
        socks.iter().any(|l| l.contains("proto=socks5 cmd=CONNECT")),
        "{socks:?}"
    );
    // HTTP and SOCKS records go to their own files.
    assert!(http.iter().all(|l| l.contains("kind=http")), "{http:?}");
    assert!(socks.iter().all(|l| l.contains("kind=socks")), "{socks:?}");

    token.cancel();
    assert!(run.await.unwrap().drained);
    access.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn multiplexed_port_drops_unknown_protocols() {
    let access = AccessLogger::start(
        AccessLogConfig::new(LogLevel::Info, LogFormat::Text, None, None),
        quiet_logger(),
    )
    .await
    .unwrap();
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: port,
        ..Config::default()
    };
    let server = server_for(&cfg, &access).await;
    let addr = server.local_http_addr().unwrap();
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    // A raw TLS ClientHello or a Postgres startup packet sent straight to the proxy is
    // neither HTTP nor SOCKS: the connection is closed without a reply.
    assert!(closed_without_reply(addr, b"\x16\x03\x01\x00\x05hello").await);
    assert!(closed_without_reply(addr, b"\x00\x00\x00\x08\x04\xd2\x16\x2f").await);

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn http_port_zero_disables_http_listener() {
    let echo = spawn_echo().await;
    let access = AccessLogger::start(
        AccessLogConfig::new(LogLevel::Info, LogFormat::Text, None, None),
        quiet_logger(),
    )
    .await
    .unwrap();
    let port = free_port().await;
    let cfg = Config {
        http_port: 0,
        socks_port: port,
        ..Config::default()
    };
    cfg.validate().unwrap();
    assert!(!cfg.is_multiplexed());
    let server = server_for(&cfg, &access).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    assert_eq!(server.listeners(), vec![(ListenerRole::Socks, addr)]);
    assert_eq!(server.local_http_addr(), None);
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let mut s = socks5_connect(addr, echo).await;
    assert_eq!(roundtrip(&mut s, b"socks only").await, b"socks only");
    drop(s);
    // The SOCKS-only listener does not speak HTTP.
    assert!(closed_without_reply(addr, b"GET http://127.0.0.1/ HTTP/1.1\r\nHost: x\r\n\r\n").await);

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn socks_port_zero_disables_socks_listener() {
    let origin = spawn_origin().await;
    let access = AccessLogger::start(
        AccessLogConfig::new(LogLevel::Info, LogFormat::Text, None, None),
        quiet_logger(),
    )
    .await
    .unwrap();
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: 0,
        ..Config::default()
    };
    cfg.validate().unwrap();
    let server = server_for(&cfg, &access).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    assert_eq!(server.listeners(), vec![(ListenerRole::Http, addr)]);
    assert_eq!(server.local_socks_addr(), None);
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let resp = http_get(addr, origin, "/http-only").await;
    assert!(resp.ends_with("origin:/http-only"), "{resp}");
    // A SOCKS5 greeting on the HTTP-only listener gets no SOCKS method selection.
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out))
        .await
        .expect("connection should be closed");
    assert!(!out.starts_with(&[5, 0]), "{out:?}");

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn both_ports_zero_is_rejected() {
    let cli = CliInput::try_parse_from(["rusty-conduit", "--http-port", "0", "--socks-port", "0"])
        .unwrap();
    let err = rusty_conduit::load_config(cli).unwrap_err().0;
    assert!(matches!(err, ConfigError::NoListeners), "{err}");

    let access = AccessLogger::start(
        AccessLogConfig::new(LogLevel::Info, LogFormat::Text, None, None),
        quiet_logger(),
    )
    .await
    .unwrap();
    let cfg = Config {
        http_port: 0,
        socks_port: 0,
        ..Config::default()
    };
    let (http, socks) = build_engines(&cfg, None, &access, &quiet_logger());
    let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
    let err = Server::from_options(&cfg.server_options(), handlers, quiet_logger())
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::NoListeners), "{err}");
}

// ---------------------------------------------------------------------------
// Connection cap
// ---------------------------------------------------------------------------

async fn quiet_access() -> AccessLogger {
    AccessLogger::start(
        AccessLogConfig::new(LogLevel::Info, LogFormat::Text, None, None),
        quiet_logger(),
    )
    .await
    .unwrap()
}

/// Open a `CONNECT` tunnel, or return `None` if the proxy closed the connection first.
async fn try_https_connect(proxy: SocketAddr, target: SocketAddr) -> Option<TcpStream> {
    let mut s = TcpStream::connect(proxy).await.ok()?;
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .ok()?;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut b))
            .await
            .ok()?
            .ok()?;
        head.push(b[0]);
    }
    head.starts_with(b"HTTP/1.1 200").then_some(s)
}

#[tokio::test]
async fn max_connections_drops_excess_and_frees_slot_on_close() {
    let echo = spawn_echo().await;
    let access = quiet_access().await;
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: 0,
        max_connections: 2,
        ..Config::default()
    };
    let server = server_for(&cfg, &access).await;
    let addr = server.local_http_addr().unwrap();
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let mut first = https_connect(addr, echo).await;
    let mut second = https_connect(addr, echo).await;

    // Full: the excess connection is closed at once, with no reply and no queueing.
    let started = std::time::Instant::now();
    assert!(closed_without_reply(addr, b"GET / HTTP/1.1\r\n\r\n").await);
    assert!(started.elapsed() < Duration::from_secs(2));

    // Existing tunnels are unaffected.
    assert_eq!(roundtrip(&mut first, b"one").await, b"one");
    assert_eq!(roundtrip(&mut second, b"two").await, b"two");

    // Closing a tunnel frees its slot.
    drop(first);
    let mut third = None;
    for _ in 0..250 {
        third = try_https_connect(addr, echo).await;
        if third.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut third = third.expect("slot should be freed after a connection closes");
    assert_eq!(roundtrip(&mut third, b"three").await, b"three");

    // Full again.
    assert!(closed_without_reply(addr, b"GET / HTTP/1.1\r\n\r\n").await);

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn max_connections_is_shared_across_listeners() {
    let echo = spawn_echo().await;
    let access = quiet_access().await;
    let (http_port, socks_port) = (free_port().await, free_port().await);
    let cfg = Config {
        http_port,
        socks_port,
        max_connections: 1,
        ..Config::default()
    };
    let server = server_for(&cfg, &access).await;
    let http = server.local_http_addr().unwrap();
    let socks = server.local_socks_addr().unwrap();
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let mut held = https_connect(http, echo).await;
    assert!(closed_without_reply(socks, &[5, 1, 0]).await);
    assert_eq!(roundtrip(&mut held, b"held").await, b"held");

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn max_connections_zero_is_unlimited() {
    let echo = spawn_echo().await;
    let access = quiet_access().await;
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: 0,
        max_connections: 0,
        ..Config::default()
    };
    let server = server_for(&cfg, &access).await;
    let addr = server.local_http_addr().unwrap();
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let mut tunnels = Vec::new();
    for _ in 0..32 {
        tunnels.push(https_connect(addr, echo).await);
    }
    for s in &mut tunnels {
        assert_eq!(roundtrip(s, b"ok").await, b"ok");
    }

    token.cancel();
    assert!(run.await.unwrap().drained);
}

#[tokio::test]
async fn max_connections_above_semaphore_limit_does_not_panic() {
    let echo = spawn_echo().await;
    let access = quiet_access().await;
    let port = free_port().await;
    let cfg = Config {
        http_port: port,
        socks_port: 0,
        max_connections: usize::MAX,
        ..Config::default()
    };
    let server = server_for(&cfg, &access).await;
    let addr = server.local_http_addr().unwrap();
    let token = server.shutdown_token();
    let run = tokio::spawn(server.run());

    let mut s = https_connect(addr, echo).await;
    assert_eq!(roundtrip(&mut s, b"ok").await, b"ok");

    token.cancel();
    assert!(run.await.unwrap().drained);
}
