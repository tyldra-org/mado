//! 12 §7 pause and resume on macOS: a download cancelled with
//! `WKDownload.cancel(resumeDataHandler)` resumes from that data in a new app
//! process, as after a relaunch. The file comes from a local server that
//! sends it slowly and honours `Range`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Checks;
use crate::download::{self, Event};
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

const SIZE: usize = 6 << 20;
const CHUNK: usize = 64 << 10;
/// Set on the child process: the resume data file and the destination.
const RESUME_ENV: &str = "MADO_PROTO_RESUME";
const DEST_ENV: &str = "MADO_PROTO_DEST";

fn byte(i: usize) -> u8 {
    (i % 251) as u8
}

/// Request lines and their `Range`/`If-*` headers, one string per request.
type Log = Arc<Mutex<Vec<String>>>;

/// Serves SIZE bytes in CHUNK pieces every 25 ms (about 2.4 s in all), with
/// an ETag, and answers `Range: bytes=N-` with 206.
fn serve() -> Result<(u16, Log), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let log: Log = Arc::default();
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                let mut request = vec![line.trim().to_owned()];
                let mut start = 0;
                line.clear();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    let header = line.trim().to_owned();
                    let lower = header.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        start = v.trim_end_matches('-').parse().unwrap_or(0).min(SIZE);
                    }
                    if lower.starts_with("range") || lower.starts_with("if-") {
                        request.push(header);
                    }
                    line.clear();
                }
                if let Ok(mut s) = seen.lock() {
                    s.push(request.join(" | "));
                }
                let status = if start > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{SIZE}\r\n",
                        SIZE - 1
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".to_owned()
                };
                let head = format!(
                    "{status}Content-Type: application/octet-stream\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"m0-v1\"\r\nLast-Modified: Thu, 01 Jan 2026 00:00:00 GMT\r\nConnection: close\r\n\r\n",
                    SIZE - start
                );
                let mut out = &stream;
                if out.write_all(head.as_bytes()).is_err() {
                    return;
                }
                let mut i = start;
                while i < SIZE {
                    let end = (i + CHUNK).min(SIZE);
                    let chunk: Vec<u8> = (i..end).map(byte).collect();
                    if out.write_all(&chunk).is_err() {
                        return;
                    }
                    i = end;
                    std::thread::sleep(Duration::from_millis(25));
                }
            });
        }
    });
    Ok((port, log))
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    if let Ok(blob) = std::env::var(RESUME_ENV) {
        return resume_here(ctx, &blob);
    }
    let mut checks = Checks::new();
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let (port, log) = serve()?;
    let dir = std::env::temp_dir().join(format!("mado-proto-downloads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join("m0.bin");
    let url = format!("http://127.0.0.1:{port}/m0.bin");
    let tab = ctx.tab;
    let d = dest.clone();
    let events = on_main(&ctx.app, move || download::start(tab, &url, d))??;
    let first = events
        .recv_timeout(STEP_TIMEOUT)
        .map_err(|e| format!("no destination asked: {e}"))?;
    std::thread::sleep(Duration::from_millis(800));
    let data = on_main(&ctx.app, download::cancel_last)??
        .recv_timeout(STEP_TIMEOUT)
        .map_err(|e| format!("no cancel callback: {e}"))?;
    let after_cancel = events.recv_timeout(Duration::from_secs(2)).ok();
    let partial = std::fs::metadata(&dest).map(|m| m.len()).ok();
    checks.check(
        "cancel(resumeDataHandler) returns resume data mid-download",
        data.is_some(),
        json!({
            "resumeDataBytes": data.as_ref().map(Vec::len), "partialFileBytes": partial,
            "firstEvent": format!("{first:?}"), "afterCancel": format!("{after_cancel:?}"),
        }),
    );
    let Some(data) = data else {
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(checks.finish(json!({ "requests": *log.lock().map_err(|e| e.to_string())? })));
    };
    let blob = dir.join("resume.data");
    std::fs::write(&blob, &data).map_err(|e| e.to_string())?;

    // A new app process, as after a relaunch: its own WebKit network process.
    let out = dir.join("child.json");
    let started = Instant::now();
    let status = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args(["--exp", "downloads", "--out"])
        .arg(&out)
        .env(RESUME_ENV, &blob)
        .env(DEST_ENV, &dest)
        .status()
        .map_err(|e| e.to_string())?;
    let child: Value = std::fs::read(&out)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let content = std::fs::read(&dest).unwrap_or_default();
    let intact = content.len() == SIZE && content.iter().enumerate().all(|(i, &b)| b == byte(i));
    let requests = log.lock().map_err(|e| e.to_string())?.clone();
    let ranged = requests
        .iter()
        .skip(1)
        .any(|r| r.to_ascii_lowercase().contains("range: bytes="));
    checks.check(
        "a new app process resumes from the data and completes the file intact",
        status.success() && intact,
        json!({
            "childExit": status.code(), "fileBytes": content.len(), "expectedBytes": SIZE,
            "intact": intact, "childSeconds": started.elapsed().as_secs_f64(),
        }),
    );
    checks.check(
        "the resumed request asks only for the rest (Range)",
        ranged,
        json!(requests),
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(checks.finish(json!({
        "resumeDataBytes": data.len(), "partialFileBytes": partial,
        "requests": requests, "child": child["result"],
    })))
}

/// The child half: resume from the parent's data and wait for the end.
fn resume_here(ctx: &Ctx, blob: &str) -> Result<Value, String> {
    let mut checks = Checks::new();
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let data = std::fs::read(blob).map_err(|e| e.to_string())?;
    let dest = std::env::var(DEST_ENV).map_err(|e| e.to_string())?;
    let tab = ctx.tab;
    let started = Instant::now();
    let events = on_main(&ctx.app, move || download::resume(tab, &data, dest.into()))??;
    let mut asked = 0;
    let outcome = loop {
        match events.recv_timeout(Duration::from_secs(30)) {
            Ok(Event::Destination) => asked += 1,
            Ok(Event::Finished) => break "Finished".to_owned(),
            Ok(Event::Failed { error, resume_data }) => {
                break format!("failed: {error} (resume data: {resume_data})");
            }
            Err(e) => break format!("no end within 30 s: {e}"),
        }
    };
    checks.check(
        "resumeDownloadFromResumeData finishes in a new process",
        outcome == "Finished",
        json!({
            "outcome": outcome, "destinationAsked": asked,
            "seconds": started.elapsed().as_secs_f64(),
        }),
    );
    Ok(checks.finish(json!({})))
}
