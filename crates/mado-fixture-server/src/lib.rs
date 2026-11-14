//! The test fixture server (spec 16 §2): `tests/pages/` on every
//! `*.mado.test` host, over HTTP and HTTPS, behind one proxy port. Tabs
//! reach it through the profile's `proxyConfigurations`, so no DNS or
//! `/etc/hosts` change is needed. HTTPS uses a test CA made at start; CI
//! trusts it in the runner's keychain only. Three hosts present bad
//! certificates on purpose: `expired.mado.test`, `wrong-name.mado.test`
//! and `self-signed.mado.test`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

/// The suffix every fixture host has.
pub const DOMAIN: &str = "mado.test";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("certificate: {0}")]
    Cert(#[from] rcgen::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// A running server. Dropping it stops accepting connections.
#[derive(Debug)]
pub struct FixtureServer {
    /// The proxy: plain `http://` requests and `CONNECT host:443`.
    pub proxy: SocketAddr,
    /// The test CA, to trust in a test keychain or client.
    pub ca_pem: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Certs {
    good: Arc<CertifiedKey>,
    special: HashMap<String, Arc<CertifiedKey>>,
}

impl std::fmt::Debug for Certs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Certs(..)")
    }
}

impl ResolvesServerCert for Certs {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let name = hello.server_name().unwrap_or_default();
        Some(
            self.special
                .get(name)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&self.good)),
        )
    }
}

fn certified(cert: &rcgen::Certificate, key: &KeyPair) -> Result<Arc<CertifiedKey>, Error> {
    let der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let signer = rustls::crypto::ring::sign::any_supported_type(&der)?;
    Ok(Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(cert.der().to_vec())],
        signer,
    )))
}

fn leaf(names: &[&str]) -> Result<CertificateParams, Error> {
    let mut params =
        CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())?;
    params
        .distinguished_name
        .push(DnType::CommonName, names.first().copied().unwrap_or(DOMAIN));
    Ok(params)
}

fn certificates() -> Result<(Certs, String), Error> {
    let ca_key = KeyPair::generate()?;
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "Mado Test CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca.self_signed(&ca_key)?;
    let issuer = Issuer::from_params(&ca, &ca_key);

    let sign = |params: CertificateParams| -> Result<Arc<CertifiedKey>, Error> {
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &issuer)?;
        certified(&cert, &key)
    };
    let good = sign(leaf(&[DOMAIN, "*.mado.test"])?)?;
    let mut expired = leaf(&["expired.mado.test"])?;
    expired.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(30);
    expired.not_after = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    let self_key = KeyPair::generate()?;
    let self_signed = leaf(&["self-signed.mado.test"])?.self_signed(&self_key)?;
    let special = HashMap::from([
        ("expired.mado.test".to_owned(), sign(expired)?),
        (
            "wrong-name.mado.test".to_owned(),
            sign(leaf(&["other.example"])?)?,
        ),
        (
            "self-signed.mado.test".to_owned(),
            certified(&self_signed, &self_key)?,
        ),
    ]);
    Ok((Certs { good, special }, ca_cert.pem()))
}

#[derive(Clone)]
struct Pages(Arc<PathBuf>);

/// `/__echo` returns the request as JSON; every other path is a file under
/// the pages folder. Hosts outside `*.mado.test` get 502: tests never reach
/// the internet through the fixture proxy.
async fn handle(State(pages): State<Pages>, request: Request) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| request.uri().host())
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host != DOMAIN && !host.ends_with(".mado.test") {
        return (
            StatusCode::BAD_GATEWAY,
            "the fixture proxy serves only *.mado.test",
        )
            .into_response();
    }
    let path = request.uri().path().to_owned();
    if path == "/__echo" {
        let headers: HashMap<String, String> = request
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
            .collect();
        return axum::Json(json!({ "host": host, "path": path, "method": request.method().as_str(), "headers": headers }))
            .into_response();
    }
    let relative = path.trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    if relative.split('/').any(|part| part == "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match tokio::fs::read(pages.0.join(relative)).await {
        Ok(bytes) => {
            let kind = match Path::new(relative).extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("css") => "text/css",
                Some("json") => "application/json",
                Some("png") => "image/png",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, kind)], Body::from(bytes)).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_http<S>(stream: S, router: Router)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = TowerToHyperService::new(router);
    let _ = auto::Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

/// Reads a request head up to the blank line, byte by byte so nothing after
/// it is consumed.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return Err(std::io::Error::other("request head too long"));
        }
        head.push(stream.read_u8().await?);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

async fn connection(
    mut stream: TcpStream,
    router: Router,
    tls: TlsAcceptor,
) -> std::io::Result<()> {
    let mut first = [0u8; 8];
    let n = stream.peek(&mut first).await?;
    if &first[..n] != b"CONNECT " {
        serve_http(stream, router).await;
        return Ok(());
    }
    let head = read_head(&mut stream).await?;
    let target = head
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    if target.ends_with(":80") {
        serve_http(stream, router).await;
    } else {
        let tls_stream = tls.accept(stream).await?;
        serve_http(tls_stream, router).await;
    }
    Ok(())
}

impl FixtureServer {
    /// Serves `pages` on 127.0.0.1 at a free port.
    pub async fn start(pages: &Path) -> Result<Self, Error> {
        let (certs, ca_pem) = certificates()?;
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(certs));
        let tls = TlsAcceptor::from(Arc::new(config));
        let router = Router::new()
            .fallback(handle)
            .with_state(Pages(Arc::new(pages.to_owned())));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let proxy = listener.local_addr()?;
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (router, tls) = (router.clone(), tls.clone());
                tokio::spawn(async move {
                    let _ = connection(stream, router, tls).await;
                });
            }
        });
        Ok(Self {
            proxy,
            ca_pem,
            task,
        })
    }
}
