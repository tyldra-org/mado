//! `mado mcp` (spec 14 §4): an MCP server on stdio. Tools are the agent
//! tools of 13 §8 that the session's scopes allow, forwarded to Mado as
//! `tool.<name>`. Until Mado allows this client, the only tool is
//! `mado_status`, which says what to do.

use mado_types::tools;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::rpc::Conn;

/// The MCP protocol revision (14 §4).
const PROTOCOL: &str = "2025-06-18";
/// How Mado knows this client; never given full control (14 §2.1).
const CLIENT: &str = "mado mcp";

/// Where the connection to Mado stands.
#[derive(Debug)]
enum Link {
    Waiting(String),
    Ready { conn: Conn, scopes: Vec<String> },
}

async fn connect(profile: Option<String>) -> Link {
    let o = crate::Opts {
        profile,
        ..crate::Opts::default()
    };
    let r = match crate::profile(&o, true) {
        Ok(r) => r,
        Err(_) => return Link::Waiting("Mado is not running and did not start.".into()),
    };
    let account = format!("{}/{CLIENT}", r.id);
    let Ok(mut conn) = Conn::connect(&r.endpoint).await else {
        return Link::Waiting("Mado's control endpoint did not answer.".into());
    };
    let client = json!({ "name": CLIENT, "version": env!("CARGO_PKG_VERSION") });
    let token = crate::tokens::get(&account);
    let hello = match conn
        .call("hello", json!({ "client": client, "token": token }))
        .await
    {
        Err(e) if e.code == -32001 && token.is_some() => {
            crate::tokens::delete(&account);
            conn.call("hello", json!({ "client": client })).await
        }
        other => other,
    };
    match hello {
        Err(e) if e.code == -32003 => Link::Waiting(
            "Control is off. In Mado, open Settings › Developers and turn on Control, then call mado_status again.".into(),
        ),
        Err(e) => Link::Waiting(format!("Mado refused: {e}")),
        Ok(h) if h["pairingRequired"] == true => {
            eprintln!("mado mcp: allow \"{CLIENT}\" in Mado's window to continue…");
            loop {
                let Ok(n) = conn.notification().await else {
                    return Link::Waiting("Mado closed the connection.".into());
                };
                match n["method"].as_str() {
                    Some("pairing.approved") => {
                        crate::tokens::set(&account, n["params"]["token"].as_str().unwrap_or_default());
                        let scopes = strings(&n["params"]["scopes"]);
                        return Link::Ready { conn, scopes };
                    }
                    Some("pairing.denied") => {
                        return Link::Waiting("The person denied this app in Mado. Ask them to allow it, then call mado_status.".into());
                    }
                    _ => {}
                }
            }
        }
        Ok(h) => Link::Ready {
            conn,
            scopes: strings(&h["scopes"]),
        },
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// The tools the scopes allow (14 §4).
fn listed(scopes: &[String]) -> Vec<Value> {
    let has = |s: &str| scopes.iter().any(|x| x == s);
    tools::all()
        .into_iter()
        .filter(|t| if t.read_only { has("read") || has("act") } else { has("act") })
        .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.parameters }))
        .collect()
}

fn status_tool(message: &str) -> Value {
    json!({
        "name": "mado_status",
        "description": format!("Mado is not ready for this client: {message}"),
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
    })
}

/// An MCP tool result from Mado's answer: images as image content, the
/// rest as JSON text.
fn content(v: &Value) -> Value {
    let mut parts = Vec::new();
    if let Some(img) = v.get("image") {
        parts.push(json!({ "type": "image", "data": img["data"], "mimeType": img["mimeType"] }));
        let mut rest = v.clone();
        if let Some(o) = rest.as_object_mut() {
            o.remove("image");
        }
        parts.push(json!({ "type": "text", "text": rest.to_string() }));
    } else {
        parts.push(
            json!({ "type": "text", "text": serde_json::to_string_pretty(v).unwrap_or_default() }),
        );
    }
    json!({ "content": parts, "isError": false })
}

fn reply(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub async fn serve(profile: Option<String>) -> i32 {
    let (out, mut outbox) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(v) = outbox.recv().await {
            let line = format!("{v}\n");
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    // Pairing may wait for the person; requests are answered meanwhile.
    let (linked_tx, mut linked_rx) = mpsc::unbounded_channel::<Link>();
    let start = |tx: mpsc::UnboundedSender<Link>, p: Option<String>| {
        tokio::spawn(async move {
            let _ = tx.send(connect(p).await);
        });
    };
    start(linked_tx.clone(), profile.clone());
    let mut link = Link::Waiting("Mado is starting; call mado_status again.".into());
    let mut connecting = true;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            got = linked_rx.recv() => {
                if let Some(l) = got {
                    let became_ready = matches!(l, Link::Ready { .. });
                    link = l;
                    connecting = false;
                    if became_ready {
                        let _ = out.send(json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }));
                    }
                }
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    let _ = out.send(error(&Value::Null, -32700, "parse error"));
                    continue;
                };
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg["method"].as_str().unwrap_or_default();
                if msg.get("id").is_none() {
                    continue; // a notification, such as notifications/initialized
                }
                let answer = match method {
                    "initialize" => reply(&id, json!({
                        "protocolVersion": PROTOCOL,
                        "capabilities": { "tools": { "listChanged": true } },
                        "serverInfo": { "name": "mado", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": crate::guide::MCP,
                    })),
                    "ping" => reply(&id, json!({})),
                    "tools/list" => match &link {
                        Link::Ready { scopes, .. } => reply(&id, json!({ "tools": listed(scopes) })),
                        Link::Waiting(why) => reply(&id, json!({ "tools": [status_tool(why)] })),
                    },
                    "tools/call" => {
                        let name = msg["params"]["name"].as_str().unwrap_or_default().to_owned();
                        let args = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
                        match &mut link {
                            Link::Waiting(why) => {
                                if !connecting {
                                    connecting = true;
                                    start(linked_tx.clone(), profile.clone());
                                }
                                reply(&id, json!({ "content": [{ "type": "text", "text": why.clone() }], "isError": name != "mado_status" }))
                            }
                            Link::Ready { conn, scopes } => {
                                if !listed(scopes).iter().any(|t| t["name"] == name.as_str()) {
                                    reply(&id, json!({ "content": [{ "type": "text", "text": format!("{name} is not available to this client") }], "isError": true }))
                                } else {
                                    match conn.call(&format!("tool.{name}"), args).await {
                                        Ok(v) => reply(&id, content(&v)),
                                        Err(e) => reply(&id, json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true })),
                                    }
                                }
                            }
                        }
                    }
                    _ => error(&id, -32601, "method not found"),
                };
                let _ = out.send(answer);
            }
        }
    }
    drop(out);
    let _ = writer.await;
    0
}
