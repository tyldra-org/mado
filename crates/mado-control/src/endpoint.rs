//! The control endpoint (spec 14 §1): a Unix socket at
//! `<profile>/control.sock`, mode 0600 in a 0700 folder, that accepts only
//! peers of the same user. Newline-delimited JSON-RPC 2.0. While control is
//! off (`developer.controlEnabled`, the M1 state), the only method is
//! `launch.forward` with the launch secret.

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use mado_types::MadoError;
use mado_types::command::LaunchRequest;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::LaunchSecret;

/// Messages larger than this close the connection (spec 14 §2).
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const UNAUTHORIZED: i64 = -32001;
const CONTROL_DISABLED: i64 = -32003;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;
const INTERNAL: i64 = -32603;

pub type ForwardHandler = Arc<
    dyn Fn(LaunchRequest) -> Pin<Box<dyn Future<Output = Result<(), MadoError>> + Send>>
        + Send
        + Sync,
>;

/// A bound endpoint. Dropping it removes the socket file.
#[derive(Debug)]
pub struct Endpoint {
    /// Bound outside any runtime; `serve` hands it to tokio.
    listener: std::os::unix::net::UnixListener,
    path: PathBuf,
    /// The user that owns the socket file, which this process just made.
    uid: u32,
}

impl Endpoint {
    /// Binds `path`, replacing a stale socket left by a killed process (the
    /// caller holds the profile lock, so no live owner uses it).
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let _ = std::fs::remove_file(path);
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(path)?);
        Ok(Self {
            listener,
            path: path.to_owned(),
            uid,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Deserialize)]
struct Rpc {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForwardParams {
    secret: String,
    #[serde(flatten)]
    request: LaunchRequest,
}

/// Accepts connections until the endpoint is dropped with its task.
pub async fn serve(endpoint: Endpoint, secret: LaunchSecret, on_forward: ForwardHandler) {
    let secret = Arc::new(secret);
    let listener = match endpoint
        .listener
        .try_clone()
        .and_then(UnixListener::from_std)
    {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(error = %e, "the control endpoint did not start");
            return;
        }
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        // Only the same OS user (spec 14 §1, check 1).
        let same_user = stream.peer_cred().is_ok_and(|c| c.uid() == endpoint.uid);
        if !same_user {
            continue;
        }
        let secret = Arc::clone(&secret);
        let on_forward = Arc::clone(&on_forward);
        tokio::spawn(async move {
            let _ = connection(stream, &secret, &on_forward).await;
        });
    }
}

async fn connection(
    stream: UnixStream,
    secret: &LaunchSecret,
    on_forward: &ForwardHandler,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read);
    let mut line = String::new();
    loop {
        line.clear();
        let n = (&mut lines)
            .take(u64::try_from(MAX_MESSAGE).unwrap_or(u64::MAX))
            .read_line(&mut line)
            .await?;
        if n == 0 || !line.ends_with('\n') {
            return Ok(());
        }
        let reply = match serde_json::from_str::<Rpc>(&line) {
            Err(_) => error(Value::Null, PARSE_ERROR, "parse error"),
            Ok(rpc) => answer(rpc, secret, on_forward).await,
        };
        let mut text = reply.to_string();
        text.push('\n');
        write.write_all(text.as_bytes()).await?;
    }
}

async fn answer(rpc: Rpc, secret: &LaunchSecret, on_forward: &ForwardHandler) -> Value {
    if rpc.method != "launch.forward" {
        return error(rpc.id, CONTROL_DISABLED, "control is off");
    }
    let params: ForwardParams = match serde_json::from_value(rpc.params) {
        Ok(p) => p,
        Err(e) => return error(rpc.id, INVALID_PARAMS, &e.to_string()),
    };
    if !secret.matches_hex(&params.secret) {
        return error(rpc.id, UNAUTHORIZED, "unauthorized");
    }
    match on_forward(params.request).await {
        Ok(()) => json!({ "jsonrpc": "2.0", "id": rpc.id, "result": { "ok": true } }),
        Err(e) => {
            json!({ "jsonrpc": "2.0", "id": rpc.id, "error": { "code": INTERNAL, "message": e.message_key, "data": e } })
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
