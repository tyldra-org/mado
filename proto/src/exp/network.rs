//! M0-T09, the network and page questions that run without a prompt:
//! experiment 13 (per-profile proxy on WebKit), 16 (find and match counts),
//! WebRTC local IP hiding and `Sec-GPC` on WebKit (08). Speech recognition,
//! screen sharing, notifications and geolocation show macOS prompts and need
//! the maintainer at the Mac.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

const FIND: &str = r#"<!doctype html><title>Find fixture</title>
<p>mado one</p><p>two Mado</p><p>three mado</p><p>other words</p>"#;

fn fixture(path: &str) -> Option<String> {
    match path {
        "/find" => Some(FIND.to_owned()),
        "/plain" => Some("<!doctype html><title>Plain</title><p>plain".to_owned()),
        _ => None,
    }
}

fn eval(ctx: &Ctx, tab: usize, body: &'static str) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, body))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered")?
}

fn load(ctx: &Ctx, tab: usize, url: String) -> Result<Value, String> {
    macos::clear_navigation(tab);
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)
}

/// A stand-in HTTP proxy: records each request line it receives (a
/// `CONNECT host:port` from WebKit) and answers 502, so nothing leaves the Mac.
/// Request lines the stand-in proxy received.
type ProxyLog = Arc<Mutex<Vec<String>>>;

fn start_proxy() -> Result<(u16, ProxyLog), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_ok()
                && let Ok(mut l) = log.lock()
            {
                l.push(line.trim().to_owned());
            }
            let mut writer = &stream;
            let _ = writer.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        }
    });
    Ok((port, seen))
}

fn proxy(ctx: &Ctx, server: &Fixtures) -> Result<Value, String> {
    let (port, seen) = start_proxy()?;
    let window = ctx.window()?;
    let w = window.clone();
    let proxied = on_main(&ctx.app, move || macos::add_tab_with_store(&w, Some(port)))??;
    let w = window.clone();
    let direct = on_main(&ctx.app, move || macos::add_tab_with_store(&w, None))??;
    let via_proxy = load(ctx, proxied, "http://example.com/".into());
    let before_direct = seen.lock().map(|l| l.len()).unwrap_or(0);
    let without = load(ctx, direct, server.url("/plain"));
    let after_direct = seen.lock().map(|l| l.len()).unwrap_or(0);
    let lines = seen.lock().map(|l| l.clone()).unwrap_or_default();
    Ok(json!({
        "proxiedLoad": via_proxy.map_err(|e| json!(e)).unwrap_or_else(|e| e),
        "proxySaw": lines,
        "directLoad": without.map_err(|e| json!(e)).unwrap_or_else(|e| e),
        "directRequestsAtProxy": after_direct - before_direct,
    }))
}

fn find(ctx: &Ctx, server: &Fixtures) -> Result<Value, String> {
    let tab = ctx.tab;
    load(ctx, tab, server.url("/find"))?;
    let ask = |text: &'static str| -> Result<bool, String> {
        let rx = on_main(&ctx.app, move || macos::find(tab, text))??;
        rx.recv_timeout(STEP_TIMEOUT)
            .map_err(|_| "find never answered".to_owned())
    };
    let present = ask("mado")?;
    let selected = eval(ctx, tab, "return getSelection().toString();")?;
    let absent = ask("absent words")?;
    // What a page script counts, case-insensitively, as the `find` script would.
    let counted = eval(
        ctx,
        tab,
        "return (document.body.innerText.match(/mado/gi) || []).length;",
    )?;
    Ok(json!({
        "matchFound": present,
        "selectionAfterFind": selected,
        "absentMatchFound": absent,
        "pageScriptCount": counted,
        "publicApiCount": null,
    }))
}

fn webrtc(ctx: &Ctx) -> Result<Value, String> {
    eval(
        ctx,
        ctx.tab,
        "const pc = new RTCPeerConnection({ iceServers: [] });
         pc.createDataChannel('probe');
         const candidates = [];
         pc.onicecandidate = (e) => { if (e.candidate) candidates.push(e.candidate.candidate); };
         await pc.setLocalDescription(await pc.createOffer());
         await new Promise((r) => {
           const t = setTimeout(r, 3000);
           pc.onicegatheringstatechange = () => { if (pc.iceGatheringState === 'complete') { clearTimeout(t); r(); } };
         });
         pc.close();
         return candidates;",
    )
}

fn gpc(ctx: &Ctx, server: &Fixtures) -> Result<Value, String> {
    let tab = ctx.tab;
    load(ctx, tab, server.url("/plain"))?;
    let header = server
        .requests()
        .iter()
        .rev()
        .find(|r| r.path == "/plain")
        .map(|r| r.headers.iter().any(|(k, _)| k == "sec-gpc"));
    let navigator = eval(ctx, tab, "return navigator.globalPrivacyControl ?? null;")?;
    Ok(json!({ "secGpcHeaderSent": header, "navigatorGlobalPrivacyControl": navigator }))
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;

    let p = proxy(ctx, &server)?;
    let through = p["proxySaw"].as_array().is_some_and(|l| {
        l.iter()
            .any(|x| x.as_str().is_some_and(|s| s.contains("example.com")))
    });
    checks.check(
        "proxy: the proxied profile's traffic reaches the proxy, the other profile's does not (row 13)",
        through && p["directRequestsAtProxy"] == 0,
        p.clone(),
    );

    let f = find(ctx, &server)?;
    checks.check(
        "find: the public API finds a present string and not an absent one (row 16)",
        f["matchFound"] == true && f["absentMatchFound"] == false,
        f.clone(),
    );

    let candidates = webrtc(ctx)?;
    let hidden = candidates.as_array().is_some_and(|c| {
        c.iter().filter_map(Value::as_str).all(|c| {
            let address = c.split_whitespace().nth(4).unwrap_or("");
            !c.contains(" typ host") || address.ends_with(".local")
        })
    });
    checks.check(
        "WebRTC: host candidates show mDNS names, not local IPs (08)",
        hidden,
        candidates.clone(),
    );

    let g = gpc(ctx, &server)?;

    Ok(checks.finish(json!({ "proxy": p, "find": f, "webrtcCandidates": candidates, "gpc": g })))
}
