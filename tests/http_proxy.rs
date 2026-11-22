//! End-to-end tests for the HTTP/HTTPS proxy engine (MITM OFF): plain HTTP forwarding,
//! zero response caching, hop-by-hop header stripping, opaque `CONNECT` tunnels, and the
//! HTTP access log file in both `text` and `json` formats.
//!
//! Every test runs a real [`Server`] on an ephemeral loopback port against local mock
//! upstreams — no external network access is needed.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
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
use rusty_conduit::logging::{AccessLogConfig, AccessLogger, LogFormat, LogLevel, Logger};
use rusty_conduit::server::{ListenPlan, ProtocolHandlers, Server, ShutdownReport};
use rusty_conduit::EngineHandlers;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn quiet_logger() -> Logger {
    Logger::with_writer(LogLevel::Quiet, Box::new(std::io::sink()))
}

/// A fresh, empty, per-test temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rc-it-http-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A running proxy server (MITM OFF) with an HTTP access log file.
struct Proxy {
    addr: SocketAddr,
    token: CancellationToken,
    run: JoinHandle<ShutdownReport>,
    access: AccessLogger,
    log_path: PathBuf,
    dir: PathBuf,
}

impl Proxy {
    async fn start(name: &str, format: LogFormat) -> Self {
        let dir = temp_dir(name);
        let log_path = dir.join("access.log");
        let cfg = Config {
            log_format: format,
            http_access_log: Some(log_path.clone()),
            ..Config::default()
        };
        let logger = quiet_logger();
        let access = AccessLogger::start(
            AccessLogConfig::new(
                LogLevel::Info,
                cfg.log_format,
                cfg.http_access_log.clone(),
                None,
            ),
            logger.clone(),
        )
        .await
        .unwrap();
        let (http, socks) = build_engines(&cfg, None, &access, &logger);
        let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
        let server = Server::bind(
            ListenPlan::Separate {
                http: Some("127.0.0.1:0".parse().unwrap()),
                socks: None,
            },
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
            log_path,
            dir,
        }
    }

    /// Wait (up to 5s) until the access log file holds at least `n` lines.
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
            "expected {n} access log lines, got {:?}",
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

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Origin answering `hit <n> <method> <path> <body-len>` with a long-lived
/// `Cache-Control` header, reporting the request header names it saw in `x-seen`, and
/// sending hop-by-hop headers the proxy must strip.
struct Origin {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
}

async fn spawn_origin() -> Origin {
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
                let service = service_fn(move |req: Request<Incoming>| {
                    let counter = Arc::clone(&counter);
                    async move { Ok::<_, Infallible>(respond(req, &counter).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await;
            });
        }
    });
    Origin { addr, hits }
}

async fn respond(req: Request<Incoming>, counter: &AtomicUsize) -> Response<Full<Bytes>> {
    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
    let mut seen: Vec<String> = req.headers().keys().map(|k| k.to_string()).collect();
    seen.sort();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let uri = req.uri().to_string();
    let body = req.into_body().collect().await.unwrap().to_bytes();
    Response::builder()
        .header("cache-control", "public, max-age=3600")
        .header("expires", "Thu, 01 Jan 2099 00:00:00 GMT")
        .header("x-seen", seen.join(","))
        .header("x-origin-uri", uri)
        .header("connection", "x-origin-hop")
        .header("x-origin-hop", "secret")
        .header("keep-alive", "timeout=5")
        .header("x-origin-keep", "yes")
        .body(Full::new(Bytes::from(format!(
            "hit {n} {method} {path} {}",
            body.len()
        ))))
        .unwrap()
}

/// Raw TCP echo server.
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

/// A parsed HTTP/1.1 response read to EOF.
struct RawResponse {
    status_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Send `request` verbatim to `proxy` and read the whole response (requests use
/// `Connection: close`).
async fn raw_exchange(proxy: SocketAddr, request: &str) -> RawResponse {
    let mut stream = TcpStream::connect(proxy).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut raw))
        .await
        .expect("response timed out")
        .unwrap();
    let text = String::from_utf8(raw).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").expect("no header terminator");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap().to_string();
    let headers = lines
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.trim().to_ascii_lowercase(), v.trim().to_string())
        })
        .collect();
    RawResponse {
        status_line,
        headers,
        body: body.to_string(),
    }
}

async fn get(proxy: SocketAddr, origin: SocketAddr, path: &str) -> RawResponse {
    raw_exchange(
        proxy,
        &format!(
            "GET http://{origin}{path} HTTP/1.1\r\nHost: {origin}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forwards_plain_http_get_and_post() {
    let origin = spawn_origin().await;
    let proxy = Proxy::start("get-post", LogFormat::Text).await;

    let resp = get(proxy.addr, origin.addr, "/hello?x=1").await;
    assert_eq!(resp.status_line, "HTTP/1.1 200 OK");
    assert_eq!(resp.body, "hit 1 GET /hello 0");
    // Upstream received an origin-form URI with the Host from the absolute URI.
    assert_eq!(resp.header("x-origin-uri"), Some("/hello?x=1"));

    let body = "name=conduit&kind=proxy";
    let resp = raw_exchange(
        proxy.addr,
        &format!(
            "POST http://{o}/submit HTTP/1.1\r\nHost: {o}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
            o = origin.addr
        ),
    )
    .await;
    assert_eq!(resp.status_line, "HTTP/1.1 200 OK");
    assert_eq!(resp.body, format!("hit 2 POST /submit {}", body.len()));
    assert_eq!(origin.hits.load(Ordering::SeqCst), 2);

    let lines = proxy.wait_log_lines(2).await;
    let post = lines
        .iter()
        .find(|l| l.contains("method=POST"))
        .expect("POST record");
    assert!(post.contains(&format!("bytes_in={}", body.len())), "{post}");
    proxy.stop().await;
}

#[tokio::test]
async fn never_caches_responses() {
    let origin = spawn_origin().await;
    let proxy = Proxy::start("no-cache", LogFormat::Text).await;

    // The origin marks every response cacheable for an hour; the proxy must still forward
    // each request and return the fresh upstream counter value.
    for n in 1..=5 {
        let resp = get(proxy.addr, origin.addr, "/same").await;
        assert_eq!(resp.status_line, "HTTP/1.1 200 OK");
        assert_eq!(resp.body, format!("hit {n} GET /same 0"));
        assert_eq!(resp.header("cache-control"), Some("public, max-age=3600"));
    }
    assert_eq!(origin.hits.load(Ordering::SeqCst), 5);
    proxy.stop().await;
}

#[tokio::test]
async fn strips_hop_by_hop_headers_both_directions() {
    let origin = spawn_origin().await;
    let proxy = Proxy::start("hop", LogFormat::Text).await;

    let resp = raw_exchange(
        proxy.addr,
        &format!(
            "GET http://{o}/hop HTTP/1.1\r\nHost: {o}\r\n\
             Connection: close, x-client-hop\r\n\
             X-Client-Hop: secret\r\n\
             Proxy-Connection: keep-alive\r\n\
             Proxy-Authorization: Basic dXNlcjpwYXNz\r\n\
             Keep-Alive: timeout=5\r\n\
             TE: trailers\r\n\
             X-Client-Keep: yes\r\n\r\n",
            o = origin.addr
        ),
    )
    .await;
    assert_eq!(resp.status_line, "HTTP/1.1 200 OK");

    let seen: Vec<&str> = resp.header("x-seen").unwrap().split(',').collect();
    assert!(seen.contains(&"x-client-keep"), "{seen:?}");
    assert!(seen.contains(&"host"), "{seen:?}");
    for hop in [
        "x-client-hop",
        "proxy-connection",
        "proxy-authorization",
        "keep-alive",
        "te",
    ] {
        assert!(!seen.contains(&hop), "{hop} forwarded upstream: {seen:?}");
    }

    // Response direction: headers named by the origin's `Connection` and the fixed
    // hop-by-hop set never reach the client; end-to-end headers do.
    assert_eq!(resp.header("x-origin-keep"), Some("yes"));
    assert_eq!(resp.header("x-origin-hop"), None);
    assert_eq!(resp.header("keep-alive"), None);
    proxy.stop().await;
}

#[tokio::test]
async fn connect_tunnel_is_opaque_without_mitm() {
    let echo = spawn_echo().await;
    let proxy = Proxy::start("connect", LogFormat::Text).await;

    let mut stream = TcpStream::connect(proxy.addr).await.unwrap();
    stream
        .write_all(format!("CONNECT {echo} HTTP/1.1\r\nHost: {echo}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        stream.read_exact(&mut b).await.unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(
        head.starts_with("HTTP/1.1 200 Connection Established\r\n"),
        "{head}"
    );

    // Arbitrary (non-HTTP, even TLS-looking) bytes pass through unchanged.
    let payload = b"\x16\x03\x01opaque-bytes\x00\xff";
    stream.write_all(payload).await.unwrap();
    let mut back = vec![0u8; payload.len()];
    stream.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, payload);
    drop(stream);

    let lines = proxy.wait_log_lines(1).await;
    assert!(
        lines[0].contains("kind=http client=127.0.0.1:"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains(&format!(
            "proto=https mitm=false method=CONNECT target={echo} status=200"
        )),
        "{lines:?}"
    );
    assert!(
        lines[0].contains(&format!(
            "bytes_in={} bytes_out={}",
            payload.len(),
            payload.len()
        )),
        "{lines:?}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn connect_to_closed_port_returns_gateway_error() {
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let proxy = Proxy::start("connect-refused", LogFormat::Text).await;
    let resp = raw_exchange(
        proxy.addr,
        &format!("CONNECT {closed} HTTP/1.1\r\nHost: {closed}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    assert!(
        resp.status_line.starts_with("HTTP/1.1 502"),
        "{}",
        resp.status_line
    );
    let lines = proxy.wait_log_lines(1).await;
    assert!(lines[0].contains("method=CONNECT"), "{lines:?}");
    assert!(lines[0].contains("status=502"), "{lines:?}");
    proxy.stop().await;
}

#[tokio::test]
async fn access_log_file_text_format() {
    let origin = spawn_origin().await;
    let proxy = Proxy::start("log-text", LogFormat::Text).await;

    let resp = get(proxy.addr, origin.addr, "/logged").await;
    assert_eq!(resp.body, "hit 1 GET /logged 0");

    let lines = proxy.wait_log_lines(1).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert!(line.starts_with("ts="), "{line}");
    let keys: Vec<&str> = line
        .split(' ')
        .map(|kv| kv.split_once('=').unwrap().0)
        .collect();
    assert_eq!(
        keys,
        [
            "ts",
            "kind",
            "client",
            "proto",
            "mitm",
            "method",
            "target",
            "status",
            "bytes_in",
            "bytes_out",
            "duration_ms"
        ]
    );
    assert!(line.contains(" kind=http client=127.0.0.1:"), "{line}");
    assert!(
        line.contains(&format!(
            " proto=http mitm=false method=GET target=http://{}/logged status=200 bytes_in=0 bytes_out={}",
            origin.addr,
            resp.body.len()
        )),
        "{line}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn access_log_file_json_format() {
    let origin = spawn_origin().await;
    let proxy = Proxy::start("log-json", LogFormat::Json).await;

    let resp = get(proxy.addr, origin.addr, "/json").await;
    assert_eq!(resp.body, "hit 1 GET /json 0");

    let lines = proxy.wait_log_lines(1).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let obj = v.as_object().unwrap();
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    for k in [
        "ts",
        "kind",
        "client",
        "proto",
        "mitm",
        "method",
        "target",
        "status",
        "bytes_in",
        "bytes_out",
        "duration_ms",
    ] {
        assert!(keys.contains(&k), "missing {k}: {v}");
    }
    assert_eq!(obj.len(), 11, "{v}");
    assert!(v["ts"].as_str().unwrap().ends_with('Z'), "{v}");
    assert_eq!(v["kind"], "http");
    assert!(
        v["client"].as_str().unwrap().starts_with("127.0.0.1:"),
        "{v}"
    );
    assert_eq!(v["proto"], "http");
    assert_eq!(v["mitm"], false);
    assert_eq!(v["method"], "GET");
    assert_eq!(v["target"], format!("http://{}/json", origin.addr));
    assert_eq!(v["status"], 200);
    assert_eq!(v["bytes_in"], 0);
    assert_eq!(v["bytes_out"], resp.body.len() as u64);
    assert!(v["duration_ms"].is_u64(), "{v}");
    proxy.stop().await;
}
