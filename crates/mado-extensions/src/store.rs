//! Downloads from the Chrome Web Store (spec 09 §2.2 step 2).

use std::time::Duration;

/// The Chrome version Mado asks as: a recent stable one, updated with each
/// Mado release. The store sends nothing for some extensions to older ones.
pub const CHROMIUM_VERSION: &str = "157.0.0.0";

/// The largest package Mado downloads.
pub const MAX_PACKAGE: usize = 200 * 1024 * 1024;

/// Where a store extension's package comes from.
pub fn package_url(id: &str) -> String {
    format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={CHROMIUM_VERSION}&acceptformat=crx3&x=id%3D{id}%26installsource%3Dondemand%26uc"
    )
}

/// Whether a download may go to `url`: https on Google's update and
/// download hosts only.
pub fn allowed(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    url.scheme() == "https"
        && (host == "clients2.google.com"
            || host == "dl.google.com"
            || host == "clients2.googleusercontent.com"
            || host.ends_with(".gvt1.com"))
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("{0}")]
    Http(String),
    #[error("over {MAX_PACKAGE} bytes")]
    TooLarge,
    #[error("the store sent nothing")]
    Empty,
}

/// A client that follows redirects only to [`allowed`] URLs.
pub fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if allowed(attempt.url()) {
                attempt.follow()
            } else {
                let url = attempt.url().to_string();
                attempt.error(format!("a redirect to {url} is not allowed"))
            }
        }))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
}

/// Downloads `url`, giving up past `max` bytes.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    max: usize,
) -> Result<Vec<u8>, DownloadError> {
    let http = |e: reqwest::Error| DownloadError::Http(e.to_string());
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(http)?;
    if response
        .content_length()
        .is_some_and(|n| usize::try_from(n).map_or(true, |n| n > max))
    {
        return Err(DownloadError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(http)? {
        if body.len() + chunk.len() > max {
            return Err(DownloadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() {
        return Err(DownloadError::Empty);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_googles_download_hosts_over_https() {
        let ok = |u: &str| allowed(&reqwest::Url::parse(u).expect("url"));
        assert!(ok("https://clients2.googleusercontent.com/crx/blobs/x.crx"));
        assert!(ok("https://r1---sn-abc.gvt1.com/edgedl/x.crx"));
        assert!(ok("https://dl.google.com/x.crx"));
        assert!(!ok("http://dl.google.com/x.crx"));
        assert!(!ok("https://gvt1.com.evil.example/x.crx"));
        assert!(!ok("https://evil.example/x.crx"));
        assert!(package_url("ddkjiahejlhfcafbddmgiahcphecmpfh").contains("prodversion=157.0.0.0"));
    }

    async fn serve_once(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await;
            let _ = s.write_all(&body).await;
        });
        format!("http://127.0.0.1:{port}/x.crx")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_download_stops_past_its_limit() {
        let client = reqwest::Client::new();
        let url = serve_once(vec![7; 4096]).await;
        assert!(matches!(
            download(&client, &url, 1024).await,
            Err(DownloadError::TooLarge)
        ));
        let url = serve_once(vec![7; 100]).await;
        assert_eq!(download(&client, &url, 1024).await.expect("ok").len(), 100);
    }

    /// The real store, by hand: `cargo nextest run -p mado-extensions
    /// --run-ignored only`. uBlock Origin Lite downloads through the
    /// allowed redirects and verifies with the store's proof.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs the network"]
    async fn a_store_package_downloads_and_verifies() {
        let id = "ddkjiahejlhfcafbddmgiahcphecmpfh";
        let bytes = download(&client().expect("client"), &package_url(id), MAX_PACKAGE)
            .await
            .expect("download");
        let verified = crate::verify(
            &bytes,
            crate::Expect {
                id: Some(id),
                from_store: true,
                extra_publishers: &[],
            },
        )
        .expect("verifies");
        assert_eq!(verified.id, id);
        let dir = tempfile::tempdir().expect("dir");
        crate::unzip(verified.zip, &dir.path().join("x")).expect("unpacks");
        assert!(dir.path().join("x/manifest.json").exists());
    }
}
