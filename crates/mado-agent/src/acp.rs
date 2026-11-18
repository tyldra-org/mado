//! The ACP client (spec 13 §7.2): Mado drives a subscription CLI's agent
//! over JSON-RPC on stdio. The agent reaches the browser only through
//! Mado's MCP server, started by the agent from `session/new`, so the
//! tools' approvals, scope and audit are the Mado runtime's.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// The ACP protocol version Mado speaks.
pub const PROTOCOL_VERSION: u64 = 1;
/// After `session/cancel`, the agent gets this long before it is stopped.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// Mado's MCP server as the agent starts it.
#[derive(Debug, Clone)]
pub struct McpServer {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// What a session needs.
#[derive(Debug, Clone)]
pub struct Session {
    /// The task's folder, or the person's for `access: full` (13 §7.2).
    pub cwd: PathBuf,
    pub mcp: McpServer,
    /// The task instructions and prompt, sent as the first turn.
    pub prompt: String,
    /// `access: full`: the agent may read and write files.
    pub full: bool,
}

/// The task's steering, follow-ups and stop, shared with the core.
#[derive(Debug, Clone, Default)]
pub struct Steering {
    pub steer: Arc<Mutex<Vec<String>>>,
    pub queue: Arc<Mutex<Vec<String>>>,
    pub stop: Arc<AtomicBool>,
}

/// Something the agent wants to do that Mado decides (13 §9).
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    /// ACP's tool kind: `read`, `edit`, `delete`, `move`, `search`,
    /// `execute`, `think`, `fetch`, `other`…
    pub kind: String,
    pub title: String,
    /// The tool's input as the agent sent it.
    pub input: Value,
    /// A file outside the task folder (`fs/*`), which always asks.
    pub path: Option<PathBuf>,
}

impl Ask {
    /// One of Mado's own MCP tools: approvals happen when Mado runs it.
    pub fn is_mado_tool(&self, server: &str) -> bool {
        let t = self.title.to_lowercase();
        t.starts_with(&format!("mcp__{server}__"))
            || t.starts_with(&format!("{server}:"))
            || t.starts_with(&format!("{server}/"))
    }
}

impl Ask {
    /// The approval class a CLI agent's own action needs (13 §7.2, §9), or
    /// `None` when it goes through: Mado's MCP tools (asked when Mado runs
    /// them) and reading, searching and thinking in the task folder.
    pub fn class(&self, server: &str) -> Option<crate::safety::Class> {
        use crate::safety::Class;
        if self.is_mado_tool(server) {
            return None;
        }
        match (self.kind.as_str(), &self.path) {
            ("read" | "search" | "think" | "switch_mode", None) => None,
            ("fetch", _) => Some(Class::LeaveScope),
            ("delete", _) => Some(Class::Delete),
            ("read" | "edit" | "move", _) => Some(Class::Files),
            _ => Some(Class::Terminal),
        }
    }

    /// The URL a `fetch` goes to, for the scope check.
    pub fn url(&self) -> String {
        self.input["url"].as_str().unwrap_or_default().to_owned()
    }
}

/// Where an answer to an [`Ask`] goes.
#[derive(Debug)]
pub struct Answer {
    key: u64,
    to: mpsc::UnboundedSender<(u64, bool)>,
}

impl Answer {
    pub fn send(self, allow: bool) {
        let _ = self.to.send((self.key, allow));
    }
}

/// What the session tells the task.
#[derive(Debug)]
pub enum Event {
    /// A step line: a tool call's title or the plan.
    Step(String),
    /// A prompt turn ended (an audit `model_call`).
    TurnEnded {
        stop_reason: String,
    },
    Ask(Ask, Answer),
}

/// How a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Done(String),
    Cancelled,
    Failed(String),
}

enum Deferred {
    Permission {
        id: Value,
        options: Vec<Value>,
    },
    Read {
        id: Value,
        path: PathBuf,
        line: Option<usize>,
        limit: Option<usize>,
    },
    Write {
        id: Value,
        path: PathBuf,
        content: String,
    },
}

/// Whether `path` is inside `dir`, without `..`.
pub fn inside(path: &Path, dir: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && path.starts_with(dir)
}

fn pick(options: &[Value], allow: bool) -> Value {
    let kinds: &[&str] = if allow {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    kinds
        .iter()
        .find_map(|k| options.iter().find(|o| o["kind"] == *k))
        .and_then(|o| o.get("optionId").cloned())
        .map_or_else(
            || json!({ "outcome": { "outcome": "cancelled" } }),
            |id| json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        )
}

fn read_text(path: &Path, line: Option<usize>, limit: Option<usize>) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    if line.is_none() && limit.is_none() {
        return Ok(text);
    }
    let from = line.unwrap_or(1).saturating_sub(1);
    Ok(text
        .lines()
        .skip(from)
        .take(limit.unwrap_or(usize::MAX))
        .collect::<Vec<_>>()
        .join("\n"))
}

struct Out<W> {
    w: W,
    next: u64,
    pending: BTreeMap<u64, &'static str>,
}

impl<W: AsyncWrite + Unpin> Out<W> {
    async fn send(&mut self, v: Value) -> bool {
        let mut line = v.to_string();
        line.push('\n');
        self.w.write_all(line.as_bytes()).await.is_ok() && self.w.flush().await.is_ok()
    }

    async fn request(&mut self, method: &'static str, params: Value) -> bool {
        self.next += 1;
        self.pending.insert(self.next, method);
        let id = self.next;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await
    }

    async fn reply(&mut self, id: Value, result: Result<Value, (i64, String)>) -> bool {
        let v = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.send(v).await
    }
}

fn take(m: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    m.lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Runs a session over `read` and `write` until it ends.
pub async fn drive<R, W>(
    read: R,
    write: W,
    s: Session,
    ctl: Steering,
    events: mpsc::UnboundedSender<Event>,
) -> End
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut out = Out {
        w: write,
        next: 0,
        pending: BTreeMap::new(),
    };
    let mut lines = BufReader::new(read).lines();
    let (answers_tx, mut answers) = mpsc::unbounded_channel::<(u64, bool)>();
    let mut deferred: BTreeMap<u64, Deferred> = BTreeMap::new();
    let mut session: Option<String> = None;
    let mut text = String::new();
    let mut steering: Vec<String> = Vec::new();
    let mut stopping: Option<Instant> = None;
    let mut keys = 0u64;
    let capabilities =
        json!({ "fs": { "readTextFile": s.full, "writeTextFile": s.full }, "terminal": false });
    if !out
        .request("initialize", json!({ "protocolVersion": PROTOCOL_VERSION, "clientCapabilities": capabilities, "clientInfo": { "name": "mado", "version": env!("CARGO_PKG_VERSION") } }))
        .await
    {
        return End::Failed("the agent did not start".into());
    }
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else {
                    return if stopping.is_some() { End::Cancelled } else { End::Failed("the agent exited".into()) };
                };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let method = msg["method"].as_str().unwrap_or_default().to_owned();
                let id = msg.get("id").cloned();
                match (method.as_str(), id) {
                    ("", Some(id)) => {
                        let Some(what) = id.as_u64().and_then(|i| out.pending.remove(&i)) else { continue };
                        let failed = msg.get("error").map(|e| e["message"].as_str().unwrap_or("error").to_owned());
                        match what {
                            "initialize" => {
                                if let Some(e) = failed {
                                    return End::Failed(e);
                                }
                                let env: Vec<Value> = s.mcp.env.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect();
                                let server = json!({ "name": s.mcp.name, "command": s.mcp.command, "args": s.mcp.args, "env": env });
                                out.request("session/new", json!({ "cwd": s.cwd, "mcpServers": [server] })).await;
                            }
                            "session/new" => {
                                let Some(sid) = msg["result"]["sessionId"].as_str() else {
                                    return End::Failed(failed.unwrap_or_else(|| "no session".into()));
                                };
                                session = Some(sid.to_owned());
                                out.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": s.prompt }] })).await;
                            }
                            "session/prompt" => {
                                let reason = msg["result"]["stopReason"].as_str().unwrap_or("error").to_owned();
                                let _ = events.send(Event::TurnEnded { stop_reason: reason.clone() });
                                if stopping.is_some() {
                                    return End::Cancelled;
                                }
                                let next = if steering.is_empty() {
                                    let q = take(&ctl.queue);
                                    (!q.is_empty()).then(|| q.iter().map(|t| format!("Next, the person asks: {t}")).collect::<Vec<_>>().join("\n"))
                                } else {
                                    Some(std::mem::take(&mut steering).iter().map(|t| format!("The person adds: {t}")).collect::<Vec<_>>().join("\n"))
                                };
                                match next {
                                    Some(p) => {
                                        text.clear();
                                        let sid = session.clone().unwrap_or_default();
                                        out.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": p }] })).await;
                                    }
                                    None => {
                                        return match failed {
                                            Some(e) => End::Failed(e),
                                            None if reason == "refusal" => End::Failed("the agent refused".into()),
                                            None => End::Done(text.trim().to_owned()),
                                        };
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    ("session/update", None) => {
                        let u = &msg["params"]["update"];
                        match u["sessionUpdate"].as_str().unwrap_or_default() {
                            "agent_message_chunk" => {
                                if let Some(t) = u["content"]["text"].as_str() {
                                    text.push_str(t);
                                }
                            }
                            "tool_call" => {
                                let title = u["title"].as_str().unwrap_or("tool").to_owned();
                                let _ = events.send(Event::Step(title));
                            }
                            "plan" => {
                                if let Some(e) = u["entries"].as_array().and_then(|e| e.iter().find(|e| e["status"] != "completed")) {
                                    let _ = events.send(Event::Step(format!("Plan: {}", e["content"].as_str().unwrap_or_default())));
                                }
                            }
                            _ => {}
                        }
                    }
                    (_, None) => {}
                    ("session/request_permission", Some(id)) => {
                        let p = &msg["params"];
                        let call = &p["toolCall"];
                        let ask = Ask {
                            kind: call["kind"].as_str().unwrap_or("other").to_owned(),
                            title: call["title"].as_str().unwrap_or_default().to_owned(),
                            input: call.get("rawInput").cloned().unwrap_or(Value::Null),
                            path: None,
                        };
                        keys += 1;
                        deferred.insert(keys, Deferred::Permission { id, options: p["options"].as_array().cloned().unwrap_or_default() });
                        let _ = events.send(Event::Ask(ask, Answer { key: keys, to: answers_tx.clone() }));
                    }
                    (m @ ("fs/read_text_file" | "fs/write_text_file"), Some(id)) if s.full => {
                        let p = &msg["params"];
                        let path = PathBuf::from(p["path"].as_str().unwrap_or_default());
                        let job = if m == "fs/read_text_file" {
                            Deferred::Read { id, path: path.clone(), line: p["line"].as_u64().map(|n| n as usize), limit: p["limit"].as_u64().map(|n| n as usize) }
                        } else {
                            Deferred::Write { id, path: path.clone(), content: p["content"].as_str().unwrap_or_default().to_owned() }
                        };
                        keys += 1;
                        if inside(&path, &s.cwd) {
                            deferred.insert(keys, job);
                            let _ = answers_tx.send((keys, true));
                        } else {
                            // 13 §7.2: files outside the task folder always ask.
                            let (kind, verb) = if m == "fs/read_text_file" { ("read", "Read") } else { ("edit", "Write") };
                            let ask = Ask { kind: kind.into(), title: format!("{verb} {}", path.display()), input: Value::Null, path: Some(path) };
                            deferred.insert(keys, job);
                            let _ = events.send(Event::Ask(ask, Answer { key: keys, to: answers_tx.clone() }));
                        }
                    }
                    (_, Some(id)) => {
                        out.reply(id, Err((-32601, "method not found".into()))).await;
                    }
                }
            }
            Some((key, allow)) = answers.recv() => {
                match deferred.remove(&key) {
                    Some(Deferred::Permission { id, options }) => {
                        out.reply(id, Ok(pick(&options, allow))).await;
                    }
                    Some(Deferred::Read { id, path, line, limit }) => {
                        let r = if allow { read_text(&path, line, limit).map(|t| json!({ "content": t })).map_err(|e| (-32603, e.to_string())) } else { Err((-32000, "denied by the person".into())) };
                        out.reply(id, r).await;
                    }
                    Some(Deferred::Write { id, path, content }) => {
                        let r = if allow { std::fs::write(&path, content).map(|()| Value::Null).map_err(|e| (-32603, e.to_string())) } else { Err((-32000, "denied by the person".into())) };
                        out.reply(id, r).await;
                    }
                    None => {}
                }
            }
            _ = tick.tick() => {
                if let Some(at) = stopping {
                    if at.elapsed() > STOP_GRACE {
                        return End::Cancelled;
                    }
                    continue;
                }
                if ctl.stop.load(Ordering::SeqCst) {
                    let Some(sid) = session.clone() else { return End::Cancelled };
                    stopping = Some(Instant::now());
                    out.send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": sid } })).await;
                    continue;
                }
                // 13 §7.2 step 6: ACP has no mid-turn input, so steering
                // cancels the turn and prompts again with the text.
                if steering.is_empty() && let Some(sid) = session.clone() && out.pending.values().any(|m| *m == "session/prompt") {
                    let s = take(&ctl.steer);
                    if !s.is_empty() {
                        steering = s;
                        out.send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": sid } })).await;
                    }
                }
            }
        }
    }
}

/// Starts the agent's command in `cwd` and drives a session with it. The
/// process is killed when the session ends.
pub async fn run(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    s: Session,
    ctl: Steering,
    events: mpsc::UnboundedSender<Event>,
) -> End {
    if let Err(e) = std::fs::create_dir_all(&s.cwd) {
        return End::Failed(e.to_string());
    }
    let child = tokio::process::Command::new(program)
        .args(args)
        .envs(env.iter().cloned())
        .current_dir(&s.cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return End::Failed(format!("{}: {e}", program.display())),
    };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return End::Failed("no stdio".into());
    };
    let end = drive(stdout, stdin, s, ctl, events).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    end
}

#[cfg(test)]
mod tests;
