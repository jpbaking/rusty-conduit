//! End-to-end tests for the SOCKS engine: SOCKS4, SOCKS4a, SOCKS5 `CONNECT` (IPv4, Domain,
//! IPv6), unsupported command rejection, the SOCKS log file in `text` and `json` formats, and
//! `--socks-block-web-in-mitm` (HTTP and HTTPS blocked, raw TCP and server-speaks-first
//! protocols allowed).
//!
//! Every test runs a real [`Server`] on an ephemeral loopback port against local upstreams.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::ServerName;
use rustls::RootCertStore;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

use rusty_conduit::build_engines;
use rusty_conduit::config::Config;
use rusty_conduit::http::mitm_tunnel::client_config_with_roots;
use rusty_conduit::logging::{AccessLogConfig, AccessLogger, LogFormat, LogLevel, Logger};
use rusty_conduit::mitm::{crypto_provider, MitmState};
use rusty_conduit::server::{ListenPlan, ProtocolHandlers, Server, ShutdownReport};
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
        "rc-it-socks-{name}-{}-{}",
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

/// Write an ephemeral PEM CA into `dir` and return `(cert, key)` paths.
fn write_test_ca(dir: &Path) -> (PathBuf, PathBuf) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "Rusty Conduit SOCKS Test CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(30);
    let cert = params.self_signed(&key).unwrap();
    let (c, k) = (dir.join("ca.pem"), dir.join("ca.key"));
    std::fs::write(&c, cert.pem()).unwrap();
    std::fs::write(&k, key.serialize_pem()).unwrap();
    (c, k)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// MITM OFF (plain transparent SOCKS).
    Plain,
    /// MITM OFF but `--socks-block-web-in-mitm` set (flag must be inactive).
    FlagWithoutMitm,
    /// MITM ON + `--socks-block-web-in-mitm`.
    BlockWeb,
}

struct SocksServer {
    addr: SocketAddr,
    token: CancellationToken,
    run: JoinHandle<ShutdownReport>,
    access: AccessLogger,
    log_path: PathBuf,
    dir: PathBuf,
}

impl SocksServer {
    async fn start(name: &str, format: LogFormat, mode: Mode) -> Self {
        let dir = temp_dir(name);
        let log_path = dir.join("socks.log");
        let mut cfg = Config {
            log_format: format,
            socks_log: Some(log_path.clone()),
            socks_block_web_in_mitm: mode != Mode::Plain,
            ..Config::default()
        };
        let state = if mode == Mode::BlockWeb {
            let (cert, key) = write_test_ca(&dir);
            cfg.mitm_ca_cert = Some(cert);
            cfg.mitm_ca_key = Some(key);
            let provider = crypto_provider();
            let upstream =
                client_config_with_roots(Arc::clone(&provider), RootCertStore::empty()).unwrap();
            Some(
                MitmState::build(
                    &cfg.mitm_options(),
                    &cfg.ca_source().unwrap(),
                    provider,
                    upstream,
                )
                .unwrap(),
            )
        } else {
            None
        };
        assert_eq!(
            cfg.socks_block_web_inactive(),
            mode == Mode::FlagWithoutMitm
        );

        let logger = quiet_logger();
        let access = AccessLogger::start(
            AccessLogConfig::new(LogLevel::Info, cfg.log_format, None, cfg.socks_log.clone()),
            logger.clone(),
        )
        .await
        .unwrap();
        let (http, socks) = build_engines(&cfg, state.as_ref(), &access, &logger);
        let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
        let server = Server::bind(
            ListenPlan::Separate {
                http: None,
                socks: Some("127.0.0.1:0".parse().unwrap()),
            },
            handlers,
            logger,
        )
        .await
        .unwrap();
        let addr = server.local_socks_addr().unwrap();
        let token = server.shutdown_token();
        let run = tokio::spawn(server.run());
        Self {
            addr,
            token,
            run,
            access,
            log_path,
            dir,
        }
    }

    async fn wait_log_lines(&self, n: usize) -> Vec<String> {
        for _ in 0..250 {
            self.access.flush().await.unwrap();
            let lines = read_lines(&self.log_path);
            if lines.len() >= n {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "expected {n} log lines, got {:?}",
            read_lines(&self.log_path)
        );
    }

    async fn stop(self) {
        self.token.cancel();
        assert!(self.run.await.unwrap().drained);
        self.access.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// TCP upstream that optionally sends a banner first, then echoes, counting every byte it
/// receives from the proxy.
struct Upstream {
    addr: SocketAddr,
    received: Arc<AtomicUsize>,
}

async fn spawn_upstream(bind: &str, banner: Option<&'static [u8]>) -> Option<Upstream> {
    let listener = TcpListener::bind(bind).await.ok()?;
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&received);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                if let Some(b) = banner {
                    if sock.write_all(b).await.is_err() {
                        return;
                    }
                }
                let mut buf = [0u8; 4096];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            counter.fetch_add(n, Ordering::SeqCst);
                            if sock.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    Some(Upstream { addr, received })
}

async fn echo() -> Upstream {
    spawn_upstream("127.0.0.1:0", None).await.unwrap()
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

/// Read until EOF (or error); returns what arrived.
async fn read_until_closed(stream: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out))
        .await
        .expect("connection was not closed");
    out
}

/// SOCKS4/4a request bytes.
fn v4_request(cmd: u8, port: u16, ip: [u8; 4], domain: Option<&str>) -> Vec<u8> {
    let mut req = vec![4, cmd];
    req.extend_from_slice(&port.to_be_bytes());
    req.extend_from_slice(&ip);
    req.extend_from_slice(b"integration-user\0");
    if let Some(d) = domain {
        req.extend_from_slice(d.as_bytes());
        req.push(0);
    }
    req
}

async fn socks4(proxy: SocketAddr, req: &[u8]) -> (TcpStream, [u8; 8]) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(req).await.unwrap();
    let mut rep = [0u8; 8];
    s.read_exact(&mut rep).await.unwrap();
    (s, rep)
}

enum Addr<'a> {
    V4([u8; 4]),
    V6([u8; 16]),
    Domain(&'a str),
}

/// SOCKS5 No-Auth greeting + request; returns the stream and the reply code.
async fn socks5(proxy: SocketAddr, cmd: u8, addr: Addr<'_>, port: u16) -> (TcpStream, u8) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await.unwrap();
    assert_eq!(sel, [5, 0], "No-Auth must be selected");
    let mut req = vec![5, cmd, 0];
    match addr {
        Addr::V4(ip) => {
            req.push(1);
            req.extend_from_slice(&ip);
        }
        Addr::V6(ip) => {
            req.push(4);
            req.extend_from_slice(&ip);
        }
        Addr::Domain(d) => {
            req.push(3);
            req.push(u8::try_from(d.len()).unwrap());
            req.extend_from_slice(d.as_bytes());
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head[0], 5);
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        other => panic!("unexpected bound ATYP {other}"),
    };
    let mut bound = vec![0u8; rest];
    s.read_exact(&mut bound).await.unwrap();
    (s, head[1])
}

fn find<'a>(lines: &'a [String], needle: &str) -> &'a String {
    lines
        .iter()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line containing {needle:?} in {lines:?}"))
}

// ---------------------------------------------------------------------------
// Protocol coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn socks4_and_socks4a_connect() {
    let up = echo().await;
    let proxy = SocksServer::start("v4", LogFormat::Text, Mode::Plain).await;
    let port = up.addr.port();

    let (mut s, rep) = socks4(proxy.addr, &v4_request(1, port, [127, 0, 0, 1], None)).await;
    assert_eq!(rep[0..2], [0x00, 0x5A]);
    assert_eq!(
        roundtrip(&mut s, b"socks4 payload").await,
        b"socks4 payload"
    );
    drop(s);

    // SOCKS4a: DSTIP 0.0.0.x with the domain resolved by the proxy.
    let (mut s, rep) = socks4(
        proxy.addr,
        &v4_request(1, port, [0, 0, 0, 1], Some("localhost")),
    )
    .await;
    assert_eq!(rep[0..2], [0x00, 0x5A]);
    assert_eq!(
        roundtrip(&mut s, b"socks4a payload").await,
        b"socks4a payload"
    );
    drop(s);

    let lines = proxy.wait_log_lines(2).await;
    let v4 = find(&lines, "proto=socks4 ");
    assert!(
        v4.contains(&format!(
            "cmd=CONNECT target=127.0.0.1:{port} status=ok bytes_in=14 bytes_out=14"
        )),
        "{v4}"
    );
    let v4a = find(&lines, "proto=socks4a ");
    assert!(
        v4a.contains(&format!(
            "cmd=CONNECT target=localhost:{port} status=ok bytes_in=15 bytes_out=15"
        )),
        "{v4a}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn socks5_connect_ipv4_domain_and_ipv6() {
    let up = echo().await;
    let proxy = SocksServer::start("v5", LogFormat::Text, Mode::Plain).await;
    let port = up.addr.port();

    let (mut s, rep) = socks5(proxy.addr, 1, Addr::V4([127, 0, 0, 1]), port).await;
    assert_eq!(rep, 0x00);
    assert_eq!(roundtrip(&mut s, b"ipv4").await, b"ipv4");
    drop(s);

    let (mut s, rep) = socks5(proxy.addr, 1, Addr::Domain("localhost"), port).await;
    assert_eq!(rep, 0x00);
    assert_eq!(roundtrip(&mut s, b"domain").await, b"domain");
    drop(s);

    let mut expected = 2;
    // IPv6 loopback may be unavailable in some sandboxes; only then is that case skipped.
    if let Some(up6) = spawn_upstream("[::1]:0", None).await {
        let mut ip = [0u8; 16];
        ip[15] = 1;
        let (mut s, rep) = socks5(proxy.addr, 1, Addr::V6(ip), up6.addr.port()).await;
        assert_eq!(rep, 0x00);
        assert_eq!(roundtrip(&mut s, b"ipv6").await, b"ipv6");
        drop(s);
        expected += 1;
        let lines = proxy.wait_log_lines(expected).await;
        find(
            &lines,
            &format!(
                "proto=socks5 cmd=CONNECT target=[::1]:{} status=ok",
                up6.addr.port()
            ),
        );
    } else {
        eprintln!("IPv6 loopback unavailable: skipping SOCKS5 ATYP=0x04 case");
    }

    let lines = proxy.wait_log_lines(expected).await;
    find(
        &lines,
        &format!("proto=socks5 cmd=CONNECT target=127.0.0.1:{port} status=ok"),
    );
    find(
        &lines,
        &format!("proto=socks5 cmd=CONNECT target=localhost:{port} status=ok"),
    );
    proxy.stop().await;
}

#[tokio::test]
async fn unsupported_commands_are_rejected() {
    let up = echo().await;
    let proxy = SocksServer::start("unsupported", LogFormat::Text, Mode::Plain).await;
    let port = up.addr.port();

    // SOCKS5 BIND and UDP ASSOCIATE → REP 0x07 (command not supported).
    for cmd in [2u8, 3] {
        let (mut s, rep) = socks5(proxy.addr, cmd, Addr::V4([127, 0, 0, 1]), port).await;
        assert_eq!(rep, 0x07, "cmd {cmd}");
        assert!(read_until_closed(&mut s).await.is_empty());
    }
    // SOCKS4 BIND → 0x5B (rejected).
    let (mut s, rep) = socks4(proxy.addr, &v4_request(2, port, [127, 0, 0, 1], None)).await;
    assert_eq!(rep[1], 0x5B);
    assert!(read_until_closed(&mut s).await.is_empty());

    // SOCKS5 without the No-Auth method → 0x05 0xFF.
    let mut s = TcpStream::connect(proxy.addr).await.unwrap();
    s.write_all(&[5, 1, 2]).await.unwrap();
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await.unwrap();
    assert_eq!(sel, [5, 0xFF]);

    let lines = proxy.wait_log_lines(4).await;
    find(&lines, "proto=socks5 cmd=BIND target=127.0.0.1:");
    find(&lines, "proto=socks5 cmd=UDP_ASSOCIATE target=127.0.0.1:");
    find(&lines, "proto=socks4 cmd=BIND target=127.0.0.1:");
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("status=command_not_supported"))
            .count(),
        3,
        "{lines:?}"
    );
    find(&lines, "status=no_acceptable_auth");
    assert_eq!(up.received.load(Ordering::SeqCst), 0);
    proxy.stop().await;
}

// ---------------------------------------------------------------------------
// SOCKS log file formats
// ---------------------------------------------------------------------------

const SOCKS_KEYS: [&str; 10] = [
    "ts",
    "kind",
    "client",
    "proto",
    "cmd",
    "target",
    "status",
    "bytes_in",
    "bytes_out",
    "duration_ms",
];

#[tokio::test]
async fn socks_log_file_text_format() {
    let up = echo().await;
    let proxy = SocksServer::start("log-text", LogFormat::Text, Mode::Plain).await;
    let (mut s, rep) = socks5(proxy.addr, 1, Addr::V4([127, 0, 0, 1]), up.addr.port()).await;
    assert_eq!(rep, 0);
    assert_eq!(roundtrip(&mut s, b"logfmt").await, b"logfmt");
    drop(s);

    let lines = proxy.wait_log_lines(1).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let keys: Vec<&str> = lines[0]
        .split(' ')
        .map(|kv| kv.split_once('=').unwrap().0)
        .collect();
    assert_eq!(keys, SOCKS_KEYS);
    assert!(
        lines[0].contains(" kind=socks client=127.0.0.1:"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains(&format!(
            " proto=socks5 cmd=CONNECT target={} status=ok bytes_in=6 bytes_out=6 duration_ms=",
            up.addr
        )),
        "{lines:?}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn socks_log_file_json_format() {
    let up = echo().await;
    let proxy = SocksServer::start("log-json", LogFormat::Json, Mode::Plain).await;
    let (mut s, rep) = socks4(
        proxy.addr,
        &v4_request(1, up.addr.port(), [127, 0, 0, 1], None),
    )
    .await;
    assert_eq!(rep[1], 0x5A);
    assert_eq!(roundtrip(&mut s, b"ndjson!").await, b"ndjson!");
    drop(s);

    let lines = proxy.wait_log_lines(1).await;
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let obj = v.as_object().unwrap();
    assert_eq!(obj.len(), SOCKS_KEYS.len(), "{v}");
    for k in SOCKS_KEYS {
        assert!(obj.contains_key(k), "missing {k}: {v}");
    }
    assert!(v["ts"].as_str().unwrap().ends_with('Z'));
    assert_eq!(v["kind"], "socks");
    assert_eq!(v["proto"], "socks4");
    assert_eq!(v["cmd"], "CONNECT");
    assert_eq!(v["target"], up.addr.to_string());
    assert_eq!(v["status"], "ok");
    assert_eq!(v["bytes_in"], 7);
    assert_eq!(v["bytes_out"], 7);
    assert!(v["duration_ms"].is_u64());
    proxy.stop().await;
}

// ---------------------------------------------------------------------------
// --socks-block-web-in-mitm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn block_web_rejects_plain_http_over_socks() {
    let up = echo().await;
    let proxy = SocksServer::start("block-http", LogFormat::Text, Mode::BlockWeb).await;

    let (mut s, rep) = socks5(proxy.addr, 1, Addr::V4([127, 0, 0, 1]), up.addr.port()).await;
    assert_eq!(rep, 0x00);
    s.write_all(b"GET / HTTP/1.1\r\nHost: example\r\n\r\n")
        .await
        .unwrap();
    assert!(read_until_closed(&mut s).await.is_empty());

    // SOCKS4 + POST is blocked as well.
    let (mut s, rep) = socks4(
        proxy.addr,
        &v4_request(1, up.addr.port(), [127, 0, 0, 1], None),
    )
    .await;
    assert_eq!(rep[1], 0x5A);
    s.write_all(b"POST /x HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    assert!(read_until_closed(&mut s).await.is_empty());

    let lines = proxy.wait_log_lines(2).await;
    assert!(
        lines
            .iter()
            .all(|l| l.contains("status=blocked_web_in_mitm bytes_in=")),
        "{lines:?}"
    );
    find(&lines, "proto=socks5 ");
    find(&lines, "proto=socks4 ");
    // The blocked chunk was never forwarded upstream.
    assert_eq!(up.received.load(Ordering::SeqCst), 0);
    proxy.stop().await;
}

#[tokio::test]
async fn block_web_rejects_https_client_hello_over_socks() {
    let up = echo().await;
    let proxy = SocksServer::start("block-tls", LogFormat::Text, Mode::BlockWeb).await;

    let (s, rep) = socks5(proxy.addr, 1, Addr::Domain("localhost"), up.addr.port()).await;
    assert_eq!(rep, 0x00);
    let mut cfg = client_config_with_roots(crypto_provider(), RootCertStore::empty()).unwrap();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let res = tokio::time::timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(cfg)).connect(ServerName::try_from("localhost").unwrap(), s),
    )
    .await
    .expect("handshake should fail promptly");
    assert!(
        res.is_err(),
        "TLS handshake over a blocked session must fail"
    );

    let lines = proxy.wait_log_lines(1).await;
    assert!(
        lines[0].contains(&format!(
            "proto=socks5 cmd=CONNECT target=localhost:{} status=blocked_web_in_mitm",
            up.addr.port()
        )),
        "{lines:?}"
    );
    assert_eq!(up.received.load(Ordering::SeqCst), 0);
    proxy.stop().await;
}

#[tokio::test]
async fn block_web_allows_raw_tcp_and_server_speaks_first() {
    let echo_up = echo().await;
    let banner_up = spawn_upstream("127.0.0.1:0", Some(b"220 smtp.test ESMTP ready\r\n"))
        .await
        .unwrap();
    let proxy = SocksServer::start("block-allow", LogFormat::Text, Mode::BlockWeb).await;

    // Raw binary protocol: forwarded unchanged.
    let (mut s, rep) = socks5(proxy.addr, 1, Addr::V4([127, 0, 0, 1]), echo_up.addr.port()).await;
    assert_eq!(rep, 0x00);
    let raw = b"\x00\x01binary-protocol\xfe\xff";
    assert_eq!(roundtrip(&mut s, raw).await, raw);
    assert_eq!(roundtrip(&mut s, b"second chunk").await, b"second chunk");
    drop(s);

    // Server-speaks-first (SMTP-like): the banner arrives before the client sends anything.
    let (mut s, rep) = socks4(
        proxy.addr,
        &v4_request(1, banner_up.addr.port(), [127, 0, 0, 1], None),
    )
    .await;
    assert_eq!(rep[1], 0x5A);
    let banner = b"220 smtp.test ESMTP ready\r\n";
    let mut got = vec![0u8; banner.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut got))
        .await
        .expect("banner must not be delayed by the guard")
        .unwrap();
    assert_eq!(&got, banner);
    assert_eq!(
        roundtrip(&mut s, b"EHLO client\r\n").await,
        b"EHLO client\r\n"
    );
    drop(s);

    let lines = proxy.wait_log_lines(2).await;
    assert!(lines.iter().all(|l| l.contains("status=ok")), "{lines:?}");
    proxy.stop().await;
}

#[tokio::test]
async fn block_web_flag_is_inactive_without_mitm() {
    let up = echo().await;
    let proxy = SocksServer::start("flag-inactive", LogFormat::Text, Mode::FlagWithoutMitm).await;

    let (mut s, rep) = socks5(proxy.addr, 1, Addr::V4([127, 0, 0, 1]), up.addr.port()).await;
    assert_eq!(rep, 0x00);
    let req = b"GET / HTTP/1.1\r\nHost: example\r\n\r\n";
    assert_eq!(roundtrip(&mut s, req).await, req);
    drop(s);

    let lines = proxy.wait_log_lines(1).await;
    assert!(lines[0].contains("status=ok"), "{lines:?}");
    assert_eq!(up.received.load(Ordering::SeqCst), req.len());
    proxy.stop().await;
}
