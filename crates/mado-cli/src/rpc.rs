//! A JSON-RPC connection to a profile's endpoint (spec 14 §2).

use std::collections::VecDeque;
use std::path::Path;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// An error the endpoint answered.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Value,
}

impl RpcError {
    /// The CLI's exit code for it (spec 14 §3).
    pub fn exit_code(&self) -> i32 {
        match self.code {
            -32006 => 3,
            -32004..=-32001 => 4,
            -32602 => 2,
            _ => 1,
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = self.data["args"]["reason"].as_str().unwrap_or_default();
        if reason.is_empty() {
            write!(f, "{} ({})", self.message, self.code)
        } else {
            write!(f, "{} ({}): {reason}", self.message, self.code)
        }
    }
}

#[derive(Debug)]
pub struct Conn {
    lines: tokio::io::Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
    next: u64,
    /// Notifications that arrived while waiting for an answer.
    pending: VecDeque<Value>,
}

impl Conn {
    pub async fn connect(endpoint: &Path) -> std::io::Result<Self> {
        let (read, write) = UnixStream::connect(endpoint).await?.into_split();
        Ok(Self {
            lines: BufReader::new(read).lines(),
            write,
            next: 1,
            pending: VecDeque::new(),
        })
    }

    async fn read(&mut self) -> Result<Value, RpcError> {
        let gone = || RpcError {
            code: -32603,
            message: "Mado closed the connection".into(),
            data: Value::Null,
        };
        let line = self
            .lines
            .next_line()
            .await
            .map_err(|_| gone())?
            .ok_or_else(gone)?;
        serde_json::from_str(&line).map_err(|_| gone())
    }

    /// Sends a request and waits for its answer.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next;
        self.next += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.write
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .map_err(|e| RpcError {
                code: -32603,
                message: e.to_string(),
                data: Value::Null,
            })?;
        loop {
            let v = self.read().await?;
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                self.pending.push_back(v);
                continue;
            }
            if let Some(e) = v.get("error") {
                return Err(RpcError {
                    code: e["code"].as_i64().unwrap_or(-32603),
                    message: e["message"].as_str().unwrap_or("error").to_owned(),
                    data: e.get("data").cloned().unwrap_or(Value::Null),
                });
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// The next notification (`{ method, params }`).
    pub async fn notification(&mut self) -> Result<Value, RpcError> {
        if let Some(v) = self.pending.pop_front() {
            return Ok(v);
        }
        loop {
            let v = self.read().await?;
            if v.get("method").is_some() {
                return Ok(v);
            }
        }
    }
}
