//! End-to-end tests for HTTPS MITM inspection: PEM and PKCS#12 CAs, the generated leaf
//! certificate (SAN, issuer, validity window with the 5-minute grace buffers), TTL expiry
//! with lazy read-time checks and background reaping, decrypted `mitm=true` access logging,
//! and zero HTTP response caching inside the tunnel.
//!
//! Every CA here is ephemeral and generated in-process; the TLS origin is local.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, ServerConfig};
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::client::TlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_util::sync::CancellationToken;
use x509_parser::extensions::GeneralName;

use rusty_conduit::build_engines;
use rusty_conduit::config::Config;
use rusty_conduit::http::mitm_tunnel::client_config_with_roots;
use rusty_conduit::logging::{AccessLogConfig, AccessLogger, LogFormat, LogLevel, Logger};
use rusty_conduit::mitm::ca::CaMaterial;
use rusty_conduit::mitm::signer::LeafSigner;
use rusty_conduit::mitm::{crypto_provider, MitmState};
use rusty_conduit::server::{ListenPlan, ProtocolHandlers, Server, ShutdownReport};
use rusty_conduit::EngineHandlers;

// ---------------------------------------------------------------------------
// Ephemeral CAs
// ---------------------------------------------------------------------------

struct TestCa {
    cert_pem: String,
    key_pem: String,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

fn test_ca(cn: &str) -> TestCa {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(365);
    let cert = params.self_signed(&key).unwrap();
    TestCa {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
        cert_der: cert.der().to_vec(),
        key_der: key.serialize_der(),
    }
}

/// Encrypted PKCS#12 bundle holding the CA key and certificate.
fn p12_bytes(ca: &TestCa, password: &str) -> Vec<u8> {
    use p12_keystore::{
        Certificate, EncryptionAlgorithm, KeyStore, KeyStoreEntry, PrivateKey, PrivateKeyChain,
    };
    let mut ks = KeyStore::new();
    let chain = PrivateKeyChain::new(
        b"ca-key".to_vec(),
        PrivateKey::from_der(&ca.key_der).unwrap(),
        [Certificate::from_der(&ca.cert_der).unwrap()],
    );
    ks.add_entry("ca", KeyStoreEntry::PrivateKeyChain(chain));
    ks.writer(password)
        .encryption_algorithm(EncryptionAlgorithm::PbeWithHmacSha256AndAes256)
        .write()
        .unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rc-it-mitm-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn quiet_logger() -> Logger {
    Logger::with_writer(LogLevel::Quiet, Box::new(std::io::sink()))
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Local TLS origin
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FixedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCert {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// HTTPS origin on 127.0.0.1 (leaf for `localhost` signed by its own private CA) answering
/// `hit <n> <method> <path> <body-len>` with an hour-long `Cache-Control`.
struct Origin {
    addr: SocketAddr,
    ca_der: CertificateDer<'static>,
    hits: Arc<AtomicUsize>,
}

async fn spawn_origin() -> Origin {
    let ca = test_ca("Rusty Conduit Origin CA");
    let material =
        CaMaterial::from_pem(ca.cert_pem.as_bytes(), ca.key_pem.as_bytes(), None).unwrap();
    let signer = LeafSigner::new(&material, Duration::from_secs(3600), crypto_provider()).unwrap();
    let leaf = signer
        .sign_at("localhost", OffsetDateTime::now_utc())
        .unwrap();
    let mut cfg = ServerConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FixedCert(leaf.certified)));
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |req: Request<Incoming>| {
                    let counter = Arc::clone(&counter);
                    async move {
                        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        let method = req.method().to_string();
                        let path = req.uri().path().to_string();
                        let body = req.into_body().collect().await.unwrap().to_bytes();
                        Ok::<_, Infallible>(
                            Response::builder()
                                .header("cache-control", "public, max-age=3600")
                                .body(Full::new(Bytes::from(format!(
                                    "hit {n} {method} {path} {}",
                                    body.len()
                                ))))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });
    Origin {
        addr,
        ca_der: CertificateDer::from(ca.cert_der),
        hits,
    }
}

// ---------------------------------------------------------------------------
// Proxy harness (real Server + build_engines + MitmState)
// ---------------------------------------------------------------------------

enum CaKind {
    Pem,
    Pkcs12,
}

struct MitmProxy {
    addr: SocketAddr,
    ca: TestCa,
    state: MitmState,
    token: CancellationToken,
    run: JoinHandle<ShutdownReport>,
    reaper: Option<JoinHandle<()>>,
    access: AccessLogger,
    log_path: PathBuf,
    dir: PathBuf,
}

impl MitmProxy {
    /// Start a MITM proxy trusting `origin` upstream. `ttl` is applied without the CLI's
    /// 10s minimum so expiry can be observed quickly; `reap_every` spawns the reaper.
    async fn start(
        name: &str,
        origin: &Origin,
        kind: CaKind,
        format: LogFormat,
        ttl: Duration,
        reap_every: Option<Duration>,
    ) -> Self {
        let dir = temp_dir(name);
        let ca = test_ca("Rusty Conduit Integration MITM CA");
        let log_path = dir.join("access.log");
        let mut cfg = Config {
            mitm_cert_ttl: ttl,
            log_format: format,
            http_access_log: Some(log_path.clone()),
            ..Config::default()
        };
        match kind {
            CaKind::Pem => {
                let (cert, key) = (dir.join("ca.pem"), dir.join("ca.key"));
                std::fs::write(&cert, &ca.cert_pem).unwrap();
                std::fs::write(&key, &ca.key_pem).unwrap();
                cfg.mitm_ca_cert = Some(cert);
                cfg.mitm_ca_key = Some(key);
            }
            CaKind::Pkcs12 => {
                let (p12, pass) = (dir.join("ca.p12"), dir.join("ca.pass"));
                std::fs::write(&p12, p12_bytes(&ca, "integration-pass")).unwrap();
                std::fs::write(&pass, "integration-pass\n").unwrap();
                cfg.mitm_ca_p12 = Some(p12);
                cfg.mitm_ca_passphrase_file = Some(pass);
            }
        }
        assert!(cfg.mitm_enabled());

        let provider = crypto_provider();
        let mut roots = RootCertStore::empty();
        roots.add(origin.ca_der.clone()).unwrap();
        let upstream = client_config_with_roots(Arc::clone(&provider), roots).unwrap();
        let state = MitmState::build(&cfg, &cfg.ca_source().unwrap(), provider, upstream).unwrap();

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
        let (http, socks) = build_engines(&cfg, Some(&state), &access, &logger);
        let handlers: Arc<dyn ProtocolHandlers> = Arc::new(EngineHandlers::new(http, socks));
        let server = Server::bind(
            ListenPlan::Separate {
                http: Some("127.0.0.1:0".parse().unwrap()),
                socks: None,
            },
            handlers,
            logger.clone(),
        )
        .await
        .unwrap();
        let addr = server.local_http_addr().unwrap();
        let token = server.shutdown_token();
        let reaper =
            reap_every.map(|every| state.cache.spawn_reaper_every(every, token.clone(), logger));
        let run = tokio::spawn(server.run());
        Self {
            addr,
            ca,
            state,
            token,
            run,
            reaper,
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

    /// `CONNECT target` and complete a TLS handshake that trusts only the MITM CA.
    async fn tunnel(&self, target: &str, sni: &str) -> TlsStream<TcpStream> {
        let tcp = connect_through(self.addr, target).await;
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(self.ca.cert_der.clone()))
            .unwrap();
        let mut cfg = client_config_with_roots(crypto_provider(), roots).unwrap();
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        TlsConnector::from(Arc::new(cfg))
            .connect(ServerName::try_from(sni.to_string()).unwrap(), tcp)
            .await
            .expect("client must trust the MITM leaf")
    }

    async fn stop(self) {
        self.token.cancel();
        self.run.await.unwrap();
        if let Some(r) = self.reaper {
            r.await.unwrap();
        }
        self.access.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn connect_through(proxy: SocketAddr, target: &str) -> TcpStream {
    let mut stream = TcpStream::connect(proxy).await.unwrap();
    stream
        .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).await.unwrap();
    assert_eq!(status, "HTTP/1.1 200 Connection Established\r\n");
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" {
            break;
        }
    }
    assert!(reader.buffer().is_empty());
    reader.into_inner()
}

type Sender = hyper::client::conn::http1::SendRequest<Empty<Bytes>>;

async fn http1(tls: TlsStream<TcpStream>) -> Sender {
    let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(conn);
    sender
}

async fn get(sender: &mut Sender, authority: &str, path: &str) -> String {
    let req = Request::get(path)
        .header(hyper::header::HOST, authority)
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

/// The leaf presented to the client, parsed: `(der, chain_len)`.
fn presented_leaf(tls: &TlsStream<TcpStream>) -> (Vec<u8>, usize) {
    let certs = tls.get_ref().1.peer_certificates().unwrap();
    (certs[0].to_vec(), certs.len())
}

fn serial_of(der: &[u8]) -> Vec<u8> {
    let (_, x) = x509_parser::parse_x509_certificate(der).unwrap();
    x.raw_serial().to_vec()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pem_ca_intercepts_logs_decrypted_requests_and_never_caches() {
    let origin = spawn_origin().await;
    let proxy = MitmProxy::start(
        "pem",
        &origin,
        CaKind::Pem,
        LogFormat::Text,
        Duration::from_secs(3600),
        None,
    )
    .await;
    let target = format!("localhost:{}", origin.addr.port());

    // Two keep-alive requests on one tunnel, then a second tunnel: every request reaches
    // the origin even though it marks responses cacheable for an hour.
    let mut sender = http1(proxy.tunnel(&target, "localhost").await).await;
    assert_eq!(get(&mut sender, &target, "/a").await, "hit 1 GET /a 0");
    assert_eq!(get(&mut sender, &target, "/a").await, "hit 2 GET /a 0");
    drop(sender);
    let mut sender = http1(proxy.tunnel(&target, "localhost").await).await;
    assert_eq!(get(&mut sender, &target, "/a").await, "hit 3 GET /a 0");
    drop(sender);
    assert_eq!(origin.hits.load(Ordering::SeqCst), 3);
    // ...but the leaf certificate is the one thing that IS cached (one host → one entry).
    assert_eq!(proxy.state.cache.len(), 1);

    let lines = proxy.wait_log_lines(3).await;
    let decrypted: Vec<&String> = lines.iter().filter(|l| l.contains("mitm=true")).collect();
    assert_eq!(decrypted.len(), 3, "{lines:?}");
    for line in decrypted {
        assert!(line.contains(" kind=http client=127.0.0.1:"), "{line}");
        assert!(
            line.contains(&format!(
                " proto=https mitm=true method=GET target=https://{target}/a status=200"
            )),
            "{line}"
        );
    }
    // The opaque CONNECT itself is never logged as a plain `mitm=false` tunnel.
    assert!(!lines.iter().any(|l| l.contains("mitm=false")), "{lines:?}");
    proxy.stop().await;
}

#[tokio::test]
async fn pkcs12_ca_intercepts_with_json_access_log() {
    let origin = spawn_origin().await;
    let proxy = MitmProxy::start(
        "p12",
        &origin,
        CaKind::Pkcs12,
        LogFormat::Json,
        Duration::from_secs(3600),
        None,
    )
    .await;
    assert!(
        proxy
            .state
            .ca_subject
            .contains("Rusty Conduit Integration MITM CA"),
        "{}",
        proxy.state.ca_subject
    );
    let target = format!("localhost:{}", origin.addr.port());

    let tls = proxy.tunnel(&target, "localhost").await;
    let (_, chain_len) = presented_leaf(&tls);
    assert_eq!(chain_len, 2, "leaf + CA");
    let mut sender = http1(tls).await;
    let body = get(&mut sender, &target, "/p12").await;
    assert_eq!(body, "hit 1 GET /p12 0");
    drop(sender);

    let lines = proxy.wait_log_lines(1).await;
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["kind"], "http");
    assert_eq!(v["proto"], "https");
    assert_eq!(v["mitm"], true);
    assert_eq!(v["method"], "GET");
    assert_eq!(v["target"], format!("https://{target}/p12"));
    assert_eq!(v["status"], 200);
    assert_eq!(v["bytes_out"], body.len() as u64);
    proxy.stop().await;
}

#[tokio::test]
async fn leaf_cert_has_san_issuer_and_grace_window() {
    let origin = spawn_origin().await;
    let ttl = Duration::from_secs(1800);
    let proxy = MitmProxy::start("leaf", &origin, CaKind::Pem, LogFormat::Text, ttl, None).await;
    let target = format!("localhost:{}", origin.addr.port());

    let before = OffsetDateTime::now_utc();
    let tls = proxy.tunnel(&target, "localhost").await;
    let after = OffsetDateTime::now_utc();
    let (der, _) = presented_leaf(&tls);
    let (_, leaf) = x509_parser::parse_x509_certificate(&der).unwrap();

    let san = leaf.subject_alternative_name().unwrap().unwrap();
    assert_eq!(
        san.value.general_names,
        vec![GeneralName::DNSName("localhost")]
    );
    assert!(leaf.subject().to_string().contains("CN=localhost"));
    assert!(
        leaf.issuer()
            .to_string()
            .contains("Rusty Conduit Integration MITM CA"),
        "{}",
        leaf.issuer()
    );
    assert!(!leaf.is_ca());

    // not_before = now - 5m, not_after = now + TTL + 5m (second precision on the wire).
    let nb = leaf.validity().not_before.timestamp();
    let na = leaf.validity().not_after.timestamp();
    let grace = 5 * 60;
    let ttl_s = ttl.as_secs() as i64;
    assert!(nb >= before.unix_timestamp() - grace - 1, "not_before {nb}");
    assert!(nb <= after.unix_timestamp() - grace + 1, "not_before {nb}");
    assert!(
        (na - nb - (ttl_s + 2 * grace)).abs() <= 1,
        "window {}",
        na - nb
    );
    drop(tls);
    proxy.stop().await;
}

#[tokio::test]
async fn ttl_expiry_and_background_reaper_regenerate_leaf() {
    let origin = spawn_origin().await;
    let ttl = Duration::from_secs(4);
    let proxy = MitmProxy::start(
        "reaper",
        &origin,
        CaKind::Pem,
        LogFormat::Text,
        ttl,
        Some(Duration::from_millis(100)),
    )
    .await;
    let target = format!("localhost:{}", origin.addr.port());

    let (first, _) = presented_leaf(&proxy.tunnel(&target, "localhost").await);
    assert_eq!(proxy.state.cache.len(), 1);
    // Within the TTL the cached leaf is reused.
    let (again, _) = presented_leaf(&proxy.tunnel(&target, "localhost").await);
    assert_eq!(serial_of(&first), serial_of(&again));

    // After the TTL the reaper evicts the entry...
    let mut evicted = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if proxy.state.cache.is_empty() {
            evicted = true;
            break;
        }
    }
    assert!(evicted, "reaper did not evict the expired leaf");

    // ...and the next handshake signs a brand-new leaf (new serial, new key).
    let tls = proxy.tunnel(&target, "localhost").await;
    let (fresh, _) = presented_leaf(&tls);
    assert_ne!(serial_of(&first), serial_of(&fresh));
    assert_eq!(proxy.state.cache.len(), 1);
    // The regenerated leaf still serves decrypted traffic.
    let mut sender = http1(tls).await;
    assert_eq!(
        get(&mut sender, &target, "/fresh").await,
        "hit 1 GET /fresh 0"
    );
    drop(sender);
    proxy.stop().await;
}

#[tokio::test]
async fn expired_leaf_is_never_served_between_reaper_ticks() {
    let origin = spawn_origin().await;
    let ttl = Duration::from_millis(800);
    // No reaper: expiry must be enforced lazily on read.
    let proxy = MitmProxy::start("lazy", &origin, CaKind::Pem, LogFormat::Text, ttl, None).await;
    let target = format!("localhost:{}", origin.addr.port());

    let (first, _) = presented_leaf(&proxy.tunnel(&target, "localhost").await);
    tokio::time::sleep(ttl + Duration::from_millis(300)).await;
    // Still stored (nothing reaped it), but expired → a miss.
    assert_eq!(proxy.state.cache.len(), 1);
    let (second, _) = presented_leaf(&proxy.tunnel(&target, "localhost").await);
    assert_ne!(serial_of(&first), serial_of(&second));
    assert_eq!(proxy.state.cache.len(), 1);
    proxy.stop().await;
}
