//! Experiment 19 (15 §5): an update the app downloads itself, unpacked and
//! launched, shows no second Gatekeeper warning. The update is
//! `target/MadoProto2.app` (`proto/build-app.sh MadoProto2 2`), zipped and
//! served from a local server. The run downloads it in-process, as the
//! updater does, unpacks it with `ditto`, and launches it through
//! LaunchServices in the background only if nothing quarantined it; a
//! quarantined copy is never launched, so no Gatekeeper dialog can appear.
//! For comparison it also downloads the zip through `WKDownload`, the path
//! of a user's download.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use super::Checks;
use crate::download::{self, Event};
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

fn serve(body: Vec<u8>) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let mut out = &stream;
            let _ = out
                .write_all(head.as_bytes())
                .and_then(|()| out.write_all(&body));
        }
    });
    Ok(port)
}

/// A plain HTTP GET in this process, as the updater fetches.
fn fetch(port: u16, dest: &Path) -> Result<usize, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .write_all(b"GET /update.zip HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let start = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no HTTP header end")?
        + 4;
    std::fs::write(dest, &raw[start..]).map_err(|e| e.to_string())?;
    Ok(raw.len() - start)
}

fn quarantine(path: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(path)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Every file in the bundle that carries a quarantine flag.
fn quarantined_files(app: &Path) -> Vec<String> {
    let out = Command::new("/usr/bin/xattr")
        .args(["-r", "-l"])
        .arg(app)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter(|l| l.contains("com.apple.quarantine"))
        .map(str::to_owned)
        .collect()
}

fn output(cmd: &mut Command) -> Value {
    match cmd.output() {
        Ok(o) => json!({
            "exit": o.status.code(),
            "out": format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim(),
        }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = exe.ancestors().nth(4).ok_or("not inside an app bundle")?;
    let update = target.join("MadoProto2.app");
    if !update.exists() {
        return Err(format!(
            "{} is missing: run proto/build-app.sh MadoProto2 2",
            update.display()
        ));
    }
    let dir = std::env::temp_dir().join(format!("mado-proto-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("extracted")).map_err(|e| e.to_string())?;
    let zip = dir.join("served.zip");
    let zipped = output(
        Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&update)
            .arg(&zip),
    );
    let port = serve(std::fs::read(&zip).map_err(|e| e.to_string())?)?;

    let fetched = dir.join("update.zip");
    let bytes = fetch(port, &fetched)?;
    let extracted = output(
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(&fetched)
            .arg(dir.join("extracted")),
    );
    let app = dir.join("extracted/MadoProto2.app");
    let flagged = quarantined_files(&app);
    checks.check(
        "the update the app downloads and unpacks carries no quarantine flag",
        quarantine(&fetched).is_none() && app.exists() && flagged.is_empty(),
        json!({ "bytes": bytes, "zip": zipped, "unzip": extracted, "zipQuarantine": quarantine(&fetched), "flaggedFiles": flagged }),
    );
    let verified = output(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app),
    );
    let assessed = output(
        Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute", "-vv"])
            .arg(&app),
    );

    let launched = if flagged.is_empty() && quarantine(&fetched).is_none() {
        let out = dir.join("launched.json");
        let opened = output(
            Command::new("/usr/bin/open")
                .args(["-n", "-g", "-W"])
                .arg(&app)
                .args(["--args", "--exp", "shell", "--out"])
                .arg(&out),
        );
        let report: Value = std::fs::read(&out)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        json!({ "open": opened, "ok": report["result"]["ok"], "build": report["environment"]["build"] })
    } else {
        json!({ "skipped": "quarantined: launching could show a Gatekeeper dialog" })
    };
    checks.check(
        "the unpacked update launches through LaunchServices and runs, with no Gatekeeper step",
        launched["ok"] == true && launched["build"] == "2" && verified["exit"] == 0,
        json!({ "launch": launched, "codesignVerify": verified, "spctl": assessed }),
    );

    // A user's download through WebKit, for comparison.
    let wk_dest = dir.join("wkdownload.zip");
    let url = format!("http://127.0.0.1:{port}/update.zip");
    let tab = ctx.tab;
    let d = wk_dest.clone();
    let events = on_main(&ctx.app, move || download::start(tab, &url, d))??;
    let mut outcome = String::from("no end");
    while let Ok(event) = events.recv_timeout(Duration::from_secs(20)) {
        match event {
            Event::Destination => continue,
            Event::Finished => outcome = "finished".into(),
            Event::Failed { error, .. } => outcome = format!("failed: {error}"),
        }
        break;
    }
    let observations = json!({
        "wkDownload": { "outcome": outcome, "quarantine": quarantine(&wk_dest) },
    });
    let _ = std::fs::remove_dir_all(&dir);
    Ok(checks.finish(observations))
}
