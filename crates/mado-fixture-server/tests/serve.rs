//! The fixture server answers through its proxy over HTTP and HTTPS, and
//! the bad-certificate hosts fail the handshake for the right reason.

use std::path::Path;
use std::sync::Arc;

use mado_fixture_server::FixtureServer;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

fn pages() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/pages")
}

async fn read_all<S: tokio::io::AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

async fn https(server: &FixtureServer, host: &str, path: &str) -> Result<String, String> {
    let mut tcp = TcpStream::connect(server.proxy)
        .await
        .map_err(|e| e.to_string())?;
    tcp.write_all(format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut reply = [0u8; 39];
    tcp.read_exact(&mut reply)
        .await
        .map_err(|e| e.to_string())?;
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let mut roots = RootCertStore::empty();
    let ca = rustls::pki_types::pem::PemObject::from_pem_slice(server.ca_pem.as_bytes())
        .map_err(|e| e.to_string())?;
    roots.add(ca).map_err(|e| e.to_string())?;
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).map_err(|e| e.to_string())?;
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|e| e.to_string())?;
    tls.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(read_all(&mut tls).await)
}

#[tokio::test]
async fn plain_http_through_the_proxy() {
    let server = FixtureServer::start(&pages()).await.expect("starts");
    let mut tcp = TcpStream::connect(server.proxy).await.expect("connects");
    tcp.write_all(b"GET http://a.mado.test/index.html HTTP/1.1\r\nHost: a.mado.test\r\nConnection: close\r\n\r\n").await.expect("writes");
    let page = read_all(&mut tcp).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("Mado fixture page"));

    let mut tcp = TcpStream::connect(server.proxy).await.expect("connects");
    tcp.write_all(
        b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
    )
    .await
    .expect("writes");
    assert!(
        read_all(&mut tcp).await.starts_with("HTTP/1.1 502"),
        "no outside hosts"
    );
}

#[tokio::test]
async fn https_with_the_test_ca_and_three_bad_certificates() {
    let server = FixtureServer::start(&pages()).await.expect("starts");
    let echo = https(&server, "shop.mado.test", "/__echo")
        .await
        .expect("TLS works");
    assert!(echo.starts_with("HTTP/1.1 200"), "{echo}");
    assert!(echo.contains("\"host\":\"shop.mado.test\""));
    for (host, reason) in [
        ("expired.mado.test", "certificate expired"),
        ("wrong-name.mado.test", "not valid for name"),
        ("self-signed.mado.test", "UnknownIssuer"),
    ] {
        let err = https(&server, host, "/")
            .await
            .expect_err("bad certificate");
        assert!(err.contains(reason), "{host}: {err}");
    }
}
