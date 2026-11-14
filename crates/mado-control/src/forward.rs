//! The forwarding side (spec 01 §1.3): a second launch hands its request to
//! the profile's owner and exits.

use std::path::Path;
use std::time::Duration;

use mado_types::command::LaunchRequest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::LaunchSecret;

/// The owner must answer within this (spec 01 §1.3).
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    #[error("no owner is listening: {0}")]
    NotListening(std::io::Error),
    #[error("the owner did not answer within 3 seconds")]
    NoAnswer,
    #[error("the owner refused: {0}")]
    Refused(Value),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// Sends `launch.forward` to the endpoint at `socket`.
pub async fn forward(
    socket: &Path,
    secret: &LaunchSecret,
    request: &LaunchRequest,
) -> Result<(), ForwardError> {
    let mut params =
        serde_json::to_value(request).map_err(|e| ForwardError::Io(std::io::Error::other(e)))?;
    params["secret"] = json!(secret.to_hex());
    let message =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "launch.forward", "params": params });
    let exchange = async {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(ForwardError::NotListening)?;
        let (read, mut write) = stream.into_split();
        write.write_all(format!("{message}\n").as_bytes()).await?;
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await?;
        let reply: Value =
            serde_json::from_str(&line).map_err(|e| ForwardError::Io(std::io::Error::other(e)))?;
        if reply["result"]["ok"] == true {
            Ok(())
        } else {
            Err(ForwardError::Refused(reply["error"].clone()))
        }
    };
    tokio::time::timeout(ANSWER_WITHIN, exchange)
        .await
        .map_err(|_| ForwardError::NoAnswer)?
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mado_types::command::LaunchSource;

    use super::*;
    use crate::{Endpoint, ForwardHandler, serve};

    #[tokio::test]
    async fn a_second_launch_reaches_the_owner_only_with_the_secret() {
        let dir = tempfile::tempdir().expect("dir");
        let socket = dir.path().join("profile/control.sock");
        let secret =
            LaunchSecret::load_or_create(&dir.path().join("launch.secret")).expect("secret");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let handler: ForwardHandler = Arc::new(move |req| {
            sink.lock().expect("lock").push(req);
            Box::pin(async { Ok(()) })
        });
        let endpoint = Endpoint::bind(&socket).expect("binds");
        let server = tokio::spawn(serve(endpoint, secret.clone(), handler));
        let request = LaunchRequest {
            urls: vec!["https://example.com/".into()],
            files: vec![],
            new_window: false,
            private: false,
            source: LaunchSource::Link,
        };

        forward(&socket, &secret, &request).await.expect("forwards");
        assert_eq!(*seen.lock().expect("lock"), vec![request.clone()]);

        let other = LaunchSecret::load_or_create(&dir.path().join("other.secret")).expect("secret");
        assert!(matches!(
            forward(&socket, &other, &request).await,
            Err(ForwardError::Refused(_))
        ));
        assert_eq!(
            seen.lock().expect("lock").len(),
            1,
            "a wrong secret changes nothing"
        );

        // Anything but launch.forward is refused while control is off.
        let stream = UnixStream::connect(&socket).await.expect("connects");
        let (read, mut write) = stream.into_split();
        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"state.get\"}\n")
            .await
            .expect("writes");
        let mut line = String::new();
        BufReader::new(read)
            .read_line(&mut line)
            .await
            .expect("reads");
        let reply: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(reply["error"]["code"], -32003);

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&socket)
                .expect("meta")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        server.abort();
    }

    #[tokio::test]
    async fn forwarding_to_nobody_fails_fast() {
        let dir = tempfile::tempdir().expect("dir");
        let secret = LaunchSecret::load_or_create(&dir.path().join("s")).expect("secret");
        let request = LaunchRequest {
            urls: vec![],
            files: vec![],
            new_window: true,
            private: false,
            source: LaunchSource::Dock,
        };
        assert!(matches!(
            forward(&dir.path().join("none.sock"), &secret, &request).await,
            Err(ForwardError::NotListening(_))
        ));
    }
}
