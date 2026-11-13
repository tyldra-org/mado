//! Experiment 20 (11 §8): the 1Password (`op`) and Bitwarden (`bw`) CLIs,
//! called by the self-signed Mado. Every call has a time limit, and the one
//! that can show 1Password's approval prompt (`op item list`, when the
//! desktop app's CLI integration is on) runs only with
//! MADO_PROTO_SUPERVISED set. `bw serve` is started on a loopback port and
//! asked for `/status`, then stopped.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Checks;
use crate::shell::Ctx;

const SUPERVISED: &str = "MADO_PROTO_SUPERVISED";

fn find(tool: &str) -> Option<String> {
    ["/opt/homebrew/bin", "/usr/local/bin"]
        .iter()
        .map(|d| format!("{d}/{tool}"))
        .find(|p| std::path::Path::new(p).exists())
}

/// Runs `tool args` with a time limit; kills it when the limit passes.
fn call(tool: &str, args: &[&str], limit: Duration) -> Value {
    let Some(path) = find(tool) else {
        return json!({ "error": format!("{tool} not installed") });
    };
    let child = Command::new(&path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                let mut err = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut err);
                }
                return json!({
                    "exit": status.code(), "seconds": started.elapsed().as_secs_f64(),
                    "stdout": out.trim(), "stderr": err.trim(),
                });
            }
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({ "timedOut": true, "seconds": limit.as_secs_f64() });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return json!({ "error": e.to_string() }),
        }
    }
}

fn get(port: u16, path: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    Ok(raw.split("\r\n\r\n").nth(1).unwrap_or("").to_owned())
}

pub fn run(_ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let limit = Duration::from_secs(20);

    let op_version = call("op", &["--version"], limit);
    let accounts = call("op", &["account", "list", "--format=json"], limit);
    let whoami = call("op", &["whoami", "--format=json"], limit);
    let account_list: Vec<Value> = accounts["stdout"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    checks.check(
        "op runs when the self-signed Mado starts it",
        op_version["exit"] == 0,
        json!({ "version": op_version, "accountList": accounts, "whoami": whoami }),
    );
    let item_list = if std::env::var_os(SUPERVISED).is_some() {
        let r = call(
            "op",
            &["item", "list", "--format=json"],
            Duration::from_secs(60),
        );
        checks.check(
            "op item list works when called by Mado (row 20)",
            r["exit"] == 0,
            r.clone(),
        );
        r
    } else {
        json!({ "skipped": "can show 1Password's approval prompt; set MADO_PROTO_SUPERVISED", "accounts": account_list.len() })
    };

    let bw_version = call("bw", &["--version"], limit);
    let bw_status = call("bw", &["status"], limit);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    let served = match find("bw") {
        Some(path) => {
            let mut child = Command::new(path)
                .args([
                    "serve",
                    "--hostname",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            let started = Instant::now();
            let mut answer = Err("never answered".to_owned());
            let mut exited = None;
            while started.elapsed() < limit {
                answer = get(port, "/status");
                if answer.is_ok() {
                    break;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    exited = Some(status.code());
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let locked = get(port, "/list/object/items");
            let _ = child.kill();
            let _ = child.wait();
            let mut said = String::new();
            for pipe in [
                child.stdout.take().map(|s| Box::new(s) as Box<dyn Read>),
                child.stderr.take().map(|s| Box::new(s) as Box<dyn Read>),
            ]
            .into_iter()
            .flatten()
            {
                let mut pipe = pipe;
                let _ = pipe.read_to_string(&mut said);
            }
            json!({
                "seconds": started.elapsed().as_secs_f64(),
                "exitedWith": exited, "output": said.trim(),
                "status": answer.as_deref().ok().and_then(|s| serde_json::from_str::<Value>(s).ok()),
                "itemsWhileLocked": locked.ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            })
        }
        None => json!({ "error": "bw not installed" }),
    };
    checks.check(
        "bw runs when the self-signed Mado starts it and reports its state as JSON",
        bw_status["stdout"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .is_some_and(|s| s["status"].is_string()),
        json!({ "version": bw_version, "status": bw_status }),
    );
    let signed_in = bw_status["stdout"]
        .as_str()
        .is_some_and(|s| !s.contains("unauthenticated"));
    if signed_in {
        checks.check(
            "bw serve, started by Mado, answers /status on loopback",
            served["status"]["success"] == true,
            served.clone(),
        );
    }
    Ok(checks.finish(json!({ "opItemList": item_list, "bwServe": served })))
}
