//! OpenRouter's OAuth with PKCE (13 §2.2): a loopback server on
//! `127.0.0.1:<random>` takes one callback, and the code becomes an API key.

use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The loopback server gives up after this (13 §2.2).
pub const WAIT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    #[error("no answer within 10 minutes")]
    TimedOut,
    #[error("the sign-in was refused or cancelled")]
    Refused,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("OpenRouter answered {0}")]
    Status(u16),
}

/// A PKCE verifier (43 characters) and its S256 challenge.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)?;
        let verifier = BASE64URL_NOPAD.encode(&bytes);
        let challenge = BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// The page that asks the person to sign in. `site` is
/// `https://openrouter.ai` (tests pass a mock).
pub fn auth_url(site: &str, port: u16, challenge: &str) -> String {
    let callback = format!("http://127.0.0.1:{port}/callback");
    let mut url = url::Url::parse(&format!("{}/auth", site.trim_end_matches('/')))
        .unwrap_or_else(|_| url::Url::parse("https://openrouter.ai/auth").expect("a URL"));
    url.query_pairs_mut()
        .append_pair("callback_url", &callback)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.into()
}

/// A loopback listener on a random port.
pub async fn listen() -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", 0)).await
}

/// Waits for one request to `/callback` and answers it; returns its
/// `code`. Any other path is answered 404 and ignored; the server stops
/// after the callback or `wait`.
pub async fn callback(listener: TcpListener, wait: Duration) -> Result<String, OauthError> {
    tokio::time::timeout(wait, async move {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let mut buf = vec![0u8; 8192];
            let mut len = 0;
            while len < buf.len() {
                let n = stream.read(&mut buf[len..]).await?;
                if n == 0 {
                    break;
                }
                len += n;
                if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&buf[..len]);
            let target = head
                .lines()
                .next()
                .and_then(|l| l.strip_prefix("GET "))
                .and_then(|l| l.split(' ').next())
                .unwrap_or("");
            let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).ok();
            let Some(parsed) = parsed.filter(|u| u.path() == "/callback") else {
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                continue;
            };
            let code = parsed
                .query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.into_owned());
            let body = if code.is_some() {
                "<!doctype html><title>Mado</title><p>Signed in. You can close this tab.</p>"
            } else {
                "<!doctype html><title>Mado</title><p>The sign-in did not finish. You can close this tab.</p>"
            };
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
            return code.filter(|c| !c.is_empty()).ok_or(OauthError::Refused);
        }
    })
    .await
    .map_err(|_| OauthError::TimedOut)?
}

/// Trades the code for an API key at `<api>/auth/keys` (`api` is
/// `https://openrouter.ai/api/v1`).
pub async fn exchange(
    client: &reqwest::Client,
    api: &str,
    code: &str,
    verifier: &str,
) -> Result<String, OauthError> {
    let response = client
        .post(format!("{}/auth/keys", api.trim_end_matches('/')))
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(OauthError::Status(status));
    }
    let body: serde_json::Value = response.json().await?;
    body["key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .ok_or(OauthError::Refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Mock;

    #[test]
    fn the_challenge_is_the_verifiers_sha256() {
        let p = Pkce::new().expect("random");
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(
            p.challenge,
            BASE64URL_NOPAD.encode(&Sha256::digest(p.verifier.as_bytes()))
        );
        let url = auth_url("https://openrouter.ai", 4567, &p.challenge);
        assert!(url.starts_with("https://openrouter.ai/auth?callback_url=http%3A%2F%2F127.0.0.1%3A4567%2Fcallback&code_challenge="));
        assert!(url.ends_with("&code_challenge_method=S256"));
    }

    #[tokio::test]
    async fn a_callback_code_becomes_a_key() {
        let mock = Mock::start(|req| {
            assert_eq!(req.path, "/api/v1/auth/keys");
            let body: serde_json::Value = serde_json::from_str(&req.body).expect("json");
            assert_eq!(
                (body["code"].as_str(), body["code_verifier"].as_str()),
                (Some("abc"), Some("v"))
            );
            (200, r#"{"key":"sk-or-123"}"#.into())
        })
        .await;
        let listener = listen().await.expect("listen");
        let port = listener.local_addr().expect("addr").port();
        let wait = tokio::spawn(callback(listener, Duration::from_secs(5)));
        let client = crate::http::client().expect("client");
        let page = client
            .get(format!("http://127.0.0.1:{port}/favicon.ico"))
            .send()
            .await
            .expect("other paths answer");
        assert_eq!(page.status().as_u16(), 404);
        let page = client
            .get(format!("http://127.0.0.1:{port}/callback?code=abc"))
            .send()
            .await
            .expect("the callback");
        assert!(page.text().await.expect("text").contains("Signed in"));
        let code = wait.await.expect("joined").expect("a code");
        let key = exchange(&client, &format!("{}/api/v1", mock.base()), &code, "v")
            .await
            .expect("a key");
        assert_eq!(key, "sk-or-123");
    }

    #[tokio::test]
    async fn the_loopback_server_gives_up() {
        let listener = listen().await.expect("listen");
        assert!(matches!(
            callback(listener, Duration::from_millis(50)).await,
            Err(OauthError::TimedOut)
        ));
    }
}
