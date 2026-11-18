//! The control endpoint (spec 14 §1, §2): a Unix socket, mode 0600 in a
//! 0700 folder, that accepts only peers of the same user. Newline-delimited
//! JSON-RPC 2.0. `launch.forward` needs only the launch secret; everything
//! else needs a session from `hello`: a paired client's token, or a new
//! pairing the person approves. The core decides; this file is transport.

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
const PROTECTED: i64 = -32002;
const CONTROL_DISABLED: i64 = -32003;
const PAIRING_DENIED: i64 = -32004;
const NOT_FOUND: i64 = -32005;
const NEEDS_USER: i64 = -32006;
const INVALID_PARAMS: i64 = -32602;
const METHOD_NOT_FOUND: i64 = -32601;
const PARSE_ERROR: i64 = -32700;
const INTERNAL: i64 = -32603;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Who `hello` says it is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

/// How a pairing ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingDecision {
    Approved {
        client: mado_types::ClientId,
        token: String,
        scopes: Vec<String>,
    },
    Denied,
}

/// The core's answer to `hello`.
#[derive(Debug)]
pub enum HelloAnswer {
    Session {
        client: mado_types::ClientId,
        scopes: Vec<String>,
    },
    /// The pairing card is up; the decision comes later.
    Pairing {
        pairing_id: String,
        decision: tokio::sync::oneshot::Receiver<PairingDecision>,
    },
    Refused(MadoError),
}

/// Answers `hello` for a client and its token, if any.
pub type HelloFn = Arc<dyn Fn(ClientInfo, Option<String>) -> BoxFuture<HelloAnswer> + Send + Sync>;
/// Runs a method for a client's session.
pub type ClientCall = Arc<
    dyn Fn(mado_types::ClientId, String, Value) -> BoxFuture<Result<Value, MadoError>>
        + Send
        + Sync,
>;

/// Follows a task for a client: the notifications to forward.
pub type FollowFn = Arc<
    dyn Fn(
            mado_types::ClientId,
            Value,
        ) -> BoxFuture<Result<tokio::sync::mpsc::UnboundedReceiver<Value>, MadoError>>
        + Send
        + Sync,
>;

/// Paired clients' access (spec 14 §2): `hello` and calls go to the core.
#[derive(Clone)]
pub struct ControlAccess {
    pub hello: HelloFn,
    /// Runs `method` with `params` for a client's session.
    pub call: ClientCall,
    /// `task.events`: the task's notifications until it ends.
    pub follow: FollowFn,
}

impl std::fmt::Debug for ControlAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ControlAccess(..)")
    }
}

/// The JSON-RPC code for an error the core returned (spec 14 §2).
pub fn rpc_code(e: &MadoError) -> i64 {
    use mado_types::ErrorCode as C;
    match (e.code, e.message_key.as_str()) {
        (_, "error.control.unauthorized") => UNAUTHORIZED,
        (_, "error.control.disabled") => CONTROL_DISABLED,
        (_, "error.control.pairingDenied") => PAIRING_DENIED,
        (_, "error.control.needsUser") => NEEDS_USER,
        (_, "error.control.badCommand") => INVALID_PARAMS,
        (_, "error.control.unknownMethod") => METHOD_NOT_FOUND,
        (C::Protected, _) => PROTECTED,
        (C::NotFound, _) => NOT_FOUND,
        (C::InvalidInput, _) => INVALID_PARAMS,
        _ => INTERNAL,
    }
}

/// Runs a method for a session that said `hello` with the test token:
/// a command name (`settings.set`) or a test hook (`test.dropNextPatch`).
pub type CallHandler = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, MadoError>> + Send>>
        + Send
        + Sync,
>;

/// End-to-end tests' access (spec 16 §1): sessions that say `hello` with
/// this token may call commands as a control client. The app passes it only
/// in builds with the `test-hooks` feature.
#[derive(Clone)]
pub struct TestAccess {
    pub token: String,
    pub call: CallHandler,
}

impl std::fmt::Debug for TestAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestAccess(..)")
    }
}

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
pub async fn serve(
    endpoint: Endpoint,
    secret: LaunchSecret,
    on_forward: ForwardHandler,
    control: Option<ControlAccess>,
    test: Option<TestAccess>,
) {
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
        let (control, test) = (control.clone(), test.clone());
        tokio::spawn(async move {
            let _ = connection(stream, secret, on_forward, control, test).await;
        });
    }
}

/// What a connection may do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Session {
    None,
    Test,
    Client(mado_types::ClientId),
}

async fn connection(
    stream: UnixStream,
    secret: Arc<LaunchSecret>,
    on_forward: ForwardHandler,
    control: Option<ControlAccess>,
    test: Option<TestAccess>,
) -> std::io::Result<()> {
    let session = Arc::new(std::sync::Mutex::new(Session::None));
    let (read, mut write) = stream.into_split();
    // One writer: answers and notifications from many tasks.
    let (out, mut outbox) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(v) = outbox.recv().await {
            let mut text = v.to_string();
            text.push('\n');
            if write.write_all(text.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let running: Arc<
        std::sync::Mutex<std::collections::BTreeMap<String, tokio::task::AbortHandle>>,
    > = Arc::default();
    let mut lines = BufReader::new(read);
    let mut line = String::new();
    let result = loop {
        line.clear();
        let n = match (&mut lines)
            .take(u64::try_from(MAX_MESSAGE).unwrap_or(u64::MAX))
            .read_line(&mut line)
            .await
        {
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        if n == 0 || !line.ends_with('\n') {
            break Ok(());
        }
        let rpc = match serde_json::from_str::<Rpc>(&line) {
            Ok(rpc) => rpc,
            Err(_) => {
                let _ = out.send(error(Value::Null, PARSE_ERROR, "parse error"));
                continue;
            }
        };
        if rpc.method == "$/cancel" {
            let id = rpc
                .params
                .get("id")
                .map(Value::to_string)
                .unwrap_or_default();
            if let Some(job) = running.lock().ok().and_then(|mut r| r.remove(&id)) {
                job.abort();
            }
            continue;
        }
        if rpc.method == "launch.forward" {
            let _ = out.send(launch(rpc, &secret, &on_forward).await);
            continue;
        }
        if rpc.method == "hello" {
            let reply = hello(rpc, control.as_ref(), test.as_ref(), &session, &out).await;
            let _ = out.send(reply);
            continue;
        }
        let current = session.lock().map(|s| s.clone()).unwrap_or(Session::None);
        if rpc.method == "task.events"
            && let (Session::Client(client), Some(control)) = (&current, control.as_ref())
        {
            match (control.follow)(*client, rpc.params.clone()).await {
                Ok(mut rx) => {
                    let _ = out.send(
                        json!({ "jsonrpc": "2.0", "id": rpc.id, "result": { "following": true } }),
                    );
                    let out2 = out.clone();
                    tokio::spawn(async move {
                        while let Some(n) = rx.recv().await {
                            if out2.send(n).is_err() {
                                break;
                            }
                        }
                    });
                }
                Err(e) => {
                    let _ = out.send(reply(Err(e), rpc.id));
                }
            }
            continue;
        }
        let id = rpc.id.clone();
        let key = id.to_string();
        let job: BoxFuture<Value> = match current {
            Session::None => {
                let code = if control.is_some() {
                    UNAUTHORIZED
                } else {
                    CONTROL_DISABLED
                };
                let _ = out.send(error(
                    id,
                    code,
                    if control.is_some() {
                        "unauthorized"
                    } else {
                        "control is off"
                    },
                ));
                continue;
            }
            Session::Test => {
                let Some(test) = test.clone() else {
                    continue;
                };
                Box::pin(async move { reply((test.call)(rpc.method, rpc.params).await, id) })
            }
            Session::Client(client) => {
                let Some(control) = control.clone() else {
                    continue;
                };
                Box::pin(
                    async move { reply((control.call)(client, rpc.method, rpc.params).await, id) },
                )
            }
        };
        let (out2, running2, key2) = (out.clone(), Arc::clone(&running), key.clone());
        let handle = tokio::spawn(async move {
            let v = job.await;
            if let Ok(mut r) = running2.lock() {
                r.remove(&key2);
            }
            let _ = out2.send(v);
        });
        if let Ok(mut r) = running.lock() {
            r.insert(key, handle.abort_handle());
        }
    };
    if let Ok(mut r) = running.lock() {
        for (_, job) in std::mem::take(&mut *r) {
            job.abort();
        }
    }
    drop(out);
    let _ = writer.await;
    result
}

fn reply(result: Result<Value, MadoError>, id: Value) -> Value {
    match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(e) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": rpc_code(&e), "message": e.message_key, "data": e } })
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloParams {
    #[serde(default)]
    client: Option<ClientInfo>,
    #[serde(default)]
    token: Option<String>,
}

async fn hello(
    rpc: Rpc,
    control: Option<&ControlAccess>,
    test: Option<&TestAccess>,
    session: &Arc<std::sync::Mutex<Session>>,
    out: &tokio::sync::mpsc::UnboundedSender<Value>,
) -> Value {
    let params: HelloParams = serde_json::from_value(rpc.params).unwrap_or(HelloParams {
        client: None,
        token: None,
    });
    let token = params.token.unwrap_or_default();
    if test.is_some_and(|t| !token.is_empty() && constant_time_eq(&t.token, &token)) {
        if let Ok(mut s) = session.lock() {
            *s = Session::Test;
        }
        return json!({ "jsonrpc": "2.0", "id": rpc.id, "result": { "session": "test", "scopes": ["test"] } });
    }
    let Some(control) = control else {
        return error(rpc.id, UNAUTHORIZED, "unauthorized");
    };
    let Some(client) = params
        .client
        .filter(|c| !c.name.trim().is_empty() && c.name.len() <= 100)
    else {
        return error(rpc.id, INVALID_PARAMS, "hello needs client.name");
    };
    let token = (!token.is_empty()).then_some(token);
    match (control.hello)(client, token).await {
        HelloAnswer::Session { client, scopes } => {
            if let Ok(mut s) = session.lock() {
                *s = Session::Client(client);
            }
            json!({ "jsonrpc": "2.0", "id": rpc.id, "result": { "session": client, "scopes": scopes } })
        }
        HelloAnswer::Pairing {
            pairing_id,
            decision,
        } => {
            let (session, out, pid) = (Arc::clone(session), out.clone(), pairing_id.clone());
            tokio::spawn(async move {
                let note = match decision.await {
                    Ok(PairingDecision::Approved {
                        client,
                        token,
                        scopes,
                    }) => {
                        if let Ok(mut s) = session.lock() {
                            *s = Session::Client(client);
                        }
                        json!({ "jsonrpc": "2.0", "method": "pairing.approved", "params": { "pairingId": pid, "token": token, "scopes": scopes } })
                    }
                    _ => {
                        json!({ "jsonrpc": "2.0", "method": "pairing.denied", "params": { "pairingId": pid } })
                    }
                };
                let _ = out.send(note);
            });
            json!({ "jsonrpc": "2.0", "id": rpc.id, "result": { "pairingRequired": true, "pairingId": pairing_id } })
        }
        HelloAnswer::Refused(e) => reply(Err(e), rpc.id),
    }
}

async fn launch(rpc: Rpc, secret: &LaunchSecret, on_forward: &ForwardHandler) -> Value {
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

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}
