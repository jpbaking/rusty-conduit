//! End-to-end tests for destination allowlist/blocklist filtering across HTTP forwarding,
//! HTTPS `CONNECT`, SOCKS4/4a, and SOCKS5 on a real multiplexed server.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use rusty_conduit::build_engines;
use rusty_conduit::config::Config;
use rusty_conduit::filter::FilterRule;
use rusty_conduit::logging::{AccessLogConfig, AccessLogger, LogFormat, LogLevel, Logger};
use rusty_conduit::server::{ListenPlan, ProtocolHandlers, Server, ShutdownReport};
use rusty_conduit::EngineHandlers;

const LOOPBACK: [u8; 4] = [127, 0, 0, 1];

fn quiet_logger() -> Logger {
    Logger::with_writer(LogLevel::Quiet, Box::new(std::io::sink()))
}

fn rules(list: &[&str]) -> Vec<FilterRule> {
    list.iter().map(|s| s.parse().unwrap()).collect()
}

fn temp_dir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rc-it-filter-{name}-{}-{}",
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

struct Proxy {
    addr: SocketAddr,
    token: CancellationToken,
    run: JoinHandle<ShutdownReport>,
    access: AccessLogger,
    http_log: PathBuf,
    socks_log: PathBuf,
    dir: PathBuf,
}

impl Proxy {
    async fn start(name: &str, configure: impl FnOnce(&mut Config)) -> Self {
        let dir = temp_dir(name);
        let http_log = dir.join("http.log");
        let socks_log = dir.join("socks.log");
        let mut cfg = Config {
            log_format: LogFormat::Text,
            http_access_log: Some(http_log.clone()),
            socks_log: Some(socks_log.clone()),
            ..Config::default()
        };
        configure(&mut cfg);
        cfg.validate().unwrap();

        let logger = quiet_logger();
        let access = AccessLogger::start(
            AccessLogConfig::new(
                LogLevel::Info,
                cfg.log_format,
                cfg.http_access_log.clone(),
                cfg.socks_log.clone(),
            ),
            logger.clone(),
        )
        .await
        .unwrap();
        let (http, socks) = build_engines(&cfg, None, &access, &logger);
        let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
        let server = Server::bind(
            ListenPlan::Multiplexed("127.0.0.1:0".parse().unwrap()),
            handlers,
            logger,
        )
        .await
        .unwrap();
        let addr = server.local_http_addr().unwrap();
        let token = server.shutdown_token();
        let run = tokio::spawn(server.run());
        Self {
            addr,
            token,
            run,
            access,
            http_log,
            socks_log,
            dir,
        }
    }

    async fn wait_lines(&self, path: &Path, n: usize) -> Vec<String> {
        for _ in 0..250 {
            self.access.flush().await.unwrap();
            let lines = read_lines(path);
            if lines.len() >= n {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "expected {n} log lines in {path:?}, got {:?}",
            read_lines(path)
        );
    }

    async fn stop(self) {
        self.token.cancel();
        assert!(self.run.await.unwrap().drained);
        self.access.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Origin {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
}

async fn spawn_http_origin() -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let service = service_fn(move |_req: Request<Incoming>| {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"ok"))))
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await;
            });
        }
    });
    Origin { addr, hits }
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

async fn http_get_status(proxy: SocketAddr, host_port: &str) -> u16 {
    let tcp = TcpStream::connect(proxy).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .unwrap();
    tokio::spawn(conn);
    let req = Request::get(format!("http://{host_port}/"))
        .header("host", host_port)
        .body(Empty::<Bytes>::new())
        .unwrap();
    sender.send_request(req).await.unwrap().status().as_u16()
}

async fn http_connect_status(proxy: SocketAddr, target: &str) -> u16 {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = [0u8; 256];
    let n = s.read(&mut buf).await.unwrap();
    let head = std::str::from_utf8(&buf[..n]).unwrap();
    let line = head.lines().next().unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

async fn socks5_connect_ipv4(proxy: SocketAddr, ip: [u8; 4], port: u16) -> (TcpStream, u8) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await.unwrap();
    assert_eq!(sel, [5, 0]);
    let mut req = vec![5, 1, 0, 1];
    req.extend_from_slice(&ip);
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await.unwrap();
    (s, rep[1])
}

async fn socks5_connect_domain(proxy: SocketAddr, host: &str, port: u16) -> (TcpStream, u8) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await.unwrap();
    assert_eq!(sel, [5, 0]);
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await.unwrap();
    (s, rep[1])
}

async fn socks4a_connect(proxy: SocketAddr, host: &str, port: u16) -> u8 {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    let mut req = vec![4, 1];
    req.extend_from_slice(&port.to_be_bytes());
    req.extend_from_slice(&[0, 0, 0, 1, 0]);
    req.extend_from_slice(host.as_bytes());
    req.push(0);
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 8];
    s.read_exact(&mut rep).await.unwrap();
    rep[1]
}

#[tokio::test]
async fn blocklist_rejects_ips_wildcards_and_dns_resolving_to_blocked_ip() {
    let origin = spawn_http_origin().await;
    let echo = spawn_echo().await;
    let proxy = Proxy::start("blocklist", |cfg| {
        cfg.block = rules(&["127.0.0.0/8", "::1", "*.blocked.test"]);
    })
    .await;

    // Literal loopback IP is blocked across HTTP GET, HTTPS CONNECT, SOCKS5
    assert_eq!(
        http_get_status(proxy.addr, &origin.addr.to_string()).await,
        403
    );
    assert_eq!(
        http_connect_status(proxy.addr, &origin.addr.to_string()).await,
        403
    );
    let (_, rep) = socks5_connect_ipv4(proxy.addr, LOOPBACK, echo.port()).await;
    assert_eq!(rep, 0x02);

    // Wildcard domain is blocked before any DNS lookup
    let blocked_dom = format!("api.blocked.test:{}", origin.addr.port());
    assert_eq!(http_get_status(proxy.addr, &blocked_dom).await, 403);
    let (_, rep) = socks5_connect_domain(proxy.addr, "api.blocked.test", echo.port()).await;
    assert_eq!(rep, 0x02);
    assert_eq!(
        socks4a_connect(proxy.addr, "api.blocked.test", echo.port()).await,
        0x5B
    );

    // Hostname `localhost` resolves to 127.0.0.1 / ::1 and is ALSO blocked!
    let localhost_origin = format!("localhost:{}", origin.addr.port());
    assert_eq!(http_get_status(proxy.addr, &localhost_origin).await, 403);
    let (_, rep) = socks5_connect_domain(proxy.addr, "localhost", echo.port()).await;
    assert_eq!(rep, 0x02);

    // Origin was never touched
    assert_eq!(origin.hits.load(Ordering::SeqCst), 0);

    // Verify access logs recorded 403 and not_allowed
    let http_lines = proxy.wait_lines(&proxy.http_log, 4).await;
    assert!(
        http_lines.iter().all(|l| l.contains("status=403")),
        "{http_lines:?}"
    );
    let socks_lines = proxy.wait_lines(&proxy.socks_log, 4).await;
    assert!(
        socks_lines.iter().all(|l| l.contains("status=not_allowed")),
        "{socks_lines:?}"
    );

    proxy.stop().await;
}

#[tokio::test]
async fn allowlist_permits_only_listed_destinations() {
    let origin = spawn_http_origin().await;
    let echo = spawn_echo().await;
    let proxy = Proxy::start("allowlist", |cfg| {
        cfg.allow = rules(&["127.0.0.1/32"]);
    })
    .await;

    // Listed IP succeeds over HTTP GET, HTTPS CONNECT, and SOCKS5
    assert_eq!(
        http_get_status(proxy.addr, &origin.addr.to_string()).await,
        200
    );
    assert_eq!(
        http_connect_status(proxy.addr, &origin.addr.to_string()).await,
        200
    );
    let (mut s, rep) = socks5_connect_ipv4(proxy.addr, LOOPBACK, echo.port()).await;
    assert_eq!(rep, 0x00);
    s.write_all(b"ping").await.unwrap();
    s.shutdown().await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    assert_eq!(out, b"ping");

    // Unlisted IP is rejected
    let (_, rep) = socks5_connect_ipv4(proxy.addr, [10, 0, 0, 1], echo.port()).await;
    assert_eq!(rep, 0x02);
    assert_eq!(http_get_status(proxy.addr, "10.0.0.1:80").await, 403);

    // Hostname `localhost` resolves to 127.0.0.1 which IS in the IP allowlist
    let localhost_origin = format!("localhost:{}", origin.addr.port());
    assert_eq!(http_get_status(proxy.addr, &localhost_origin).await, 200);

    proxy.stop().await;
}

#[tokio::test]
async fn domain_only_allowlist_rejects_unlisted_domains_and_ips() {
    let origin = spawn_http_origin().await;
    let echo = spawn_echo().await;
    let proxy = Proxy::start("domain-allow", |cfg| {
        cfg.allow = rules(&["localhost", "*.allowed.test"]);
    })
    .await;

    // Exact allowed domain `localhost` succeeds
    let localhost_origin = format!("localhost:{}", origin.addr.port());
    assert_eq!(http_get_status(proxy.addr, &localhost_origin).await, 200);

    // Unlisted domain is rejected without dialing
    assert_eq!(http_get_status(proxy.addr, "other.test:80").await, 403);
    // Literal IP is rejected because there are no IP rules in the allowlist
    assert_eq!(
        http_get_status(proxy.addr, &origin.addr.to_string()).await,
        403
    );
    let (_, rep) = socks5_connect_ipv4(proxy.addr, LOOPBACK, echo.port()).await;
    assert_eq!(rep, 0x02);

    proxy.stop().await;
}
