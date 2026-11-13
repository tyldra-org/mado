//! M0-T07, the macOS platform questions that run without a prompt:
//! experiment 10 (Apple Pay with and without injected scripts), 17 (Safe
//! Browsing warning in WKWebView), 18 (`.webarchive` saving) and 19 (whether
//! files the app downloads get the quarantine flag that triggers
//! Gatekeeper). Experiment 11 (Keychain) can show a prompt and needs the
//! maintainer at the Mac.

use std::time::Duration;

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

/// The page asks for Apple Pay itself and reports to the server, so no
/// script of ours runs in it unless the experiment adds one.
const PAY: &str = r#"<!doctype html><title>Apple Pay fixture</title><script>
(async () => {
  let r = { has: typeof ApplePaySession !== "undefined", can: null, secure: isSecureContext };
  if (r.has) { try { r.can = ApplePaySession.canMakePayments(); } catch (e) { r.can = "threw: " + e.message; } }
  fetch("/report?" + encodeURIComponent(JSON.stringify(r)));
})();
</script>"#;

const ARCHIVE: &str = r#"<!doctype html><title>Archive fixture</title>
<style>p{color:rgb(1,2,3)}</style><p id="t">archived page text</p>"#;

fn fixture(path: &str) -> Option<String> {
    match path {
        "/pay" => Some(PAY.to_owned()),
        "/archive" => Some(ARCHIVE.to_owned()),
        p if p.starts_with("/report") => Some(String::new()),
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

/// The JSON the pay page sent to /report, the `n`th time.
fn pay_report(server: &Fixtures, n: usize) -> Result<Value, String> {
    for _ in 0..100 {
        let reports: Vec<String> = server
            .requests()
            .into_iter()
            .filter_map(|r| r.path.strip_prefix("/report?").map(str::to_owned))
            .collect();
        if let Some(raw) = reports.get(n) {
            let decoded = percent_decode(raw);
            return serde_json::from_str(&decoded).map_err(|e| e.to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("the page never reported".into())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(h) = bytes.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(h), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn apple_pay(ctx: &Ctx, server: &Fixtures) -> Result<Value, String> {
    let tab = ctx.tab;
    load(ctx, tab, server.url("/pay"))?;
    let clean = pay_report(server, 0)?;
    on_main(&ctx.app, move || {
        macos::add_user_script(tab, "/* a Mado page script */")
    })??;
    macos::clear_navigation(tab);
    on_main(&ctx.app, move || macos::reload(tab))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    let injected = pay_report(server, 1)?;
    // The same question on a real HTTPS origin, asked through our own script.
    let https = load(ctx, tab, "https://example.com/".into()).and_then(|_| {
        eval(
            ctx,
            tab,
            "const has = typeof ApplePaySession !== 'undefined';
             let can = null;
             if (has) { try { can = ApplePaySession.canMakePayments(); } catch (e) { can = 'threw: ' + e.message; } }
             return { has, can, secure: isSecureContext };",
        )
    });
    Ok(json!({
        "localhostNoScripts": clean,
        "localhostWithUserScript": injected,
        "httpsAfterEvaluate": https.unwrap_or_else(|e| json!(e)),
    }))
}

fn safe_browsing(ctx: &Ctx) -> Result<Value, String> {
    let window = ctx.window()?;
    let tab = on_main(&ctx.app, move || macos::add_tab_with_store(&window, None))??;
    on_main(&ctx.app, move || macos::show_tab(tab))??;
    let url = "http://testsafebrowsing.appspot.com/s/phishing.html";
    let navigation = load(ctx, tab, url.into());
    // WebKit shows its warning as a view over the page.
    std::thread::sleep(Duration::from_millis(1500));
    let classes = on_main(&ctx.app, move || macos::view_classes(tab))??;
    let warning: Vec<&String> = classes
        .iter()
        .filter(|c| c.contains("Warning") || c.contains("SafeBrowsing"))
        .collect();
    let title = eval(ctx, tab, "return document.title;").unwrap_or(Value::Null);
    on_main(&ctx.app, move || macos::close_tab(tab))??;
    Ok(json!({
        "url": url,
        "navigation": navigation.map_err(|e| json!(e)).unwrap_or_else(|e| e),
        "warningViews": warning,
        "pageTitle": title,
    }))
}

fn web_archive(ctx: &Ctx, server: &Fixtures) -> Result<Value, String> {
    let tab = ctx.tab;
    load(ctx, tab, server.url("/archive"))?;
    let rx = on_main(&ctx.app, move || macos::web_archive(tab))??;
    let bytes = rx.recv_timeout(STEP_TIMEOUT).map_err(|_| "no archive")??;
    let file = std::env::temp_dir().join(format!("mado-proto-{}.webarchive", std::process::id()));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let read = std::fs::read(&file).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&file);
    let window = ctx.window()?;
    let other = on_main(&ctx.app, move || macos::add_tab_with_store(&window, None))??;
    macos::clear_navigation(other);
    on_main(&ctx.app, move || {
        macos::load_data(other, &read, "application/x-webarchive", "about:blank")
    })??;
    let loaded = macos::wait_navigation(other, STEP_TIMEOUT);
    let shown = eval(
        ctx,
        other,
        "const p = document.querySelector('#t');
         return { text: p?.textContent ?? null, color: p ? getComputedStyle(p).color : null };",
    )?;
    Ok(json!({
        "bytes": bytes.len(),
        "magic": String::from_utf8_lossy(&bytes[..bytes.len().min(8)]),
        "reloadedIn": "a second WKWebView (opening it in Safari would interrupt the user)",
        "load": loaded.map_err(|e| json!(e)).unwrap_or_else(|e| e),
        "shown": shown,
    }))
}

fn quarantined(path: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(path)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn quarantine() -> Result<Value, String> {
    let dir = std::env::temp_dir().join(format!("mado-proto-download-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // As Mado's updater would: the app writes the bytes itself.
    let written = dir.join("written-by-app.bin");
    std::fs::write(&written, b"update").map_err(|e| e.to_string())?;
    // A helper process the app starts.
    let fetched = dir.join("fetched-by-curl.html");
    let status = std::process::Command::new("/usr/bin/curl")
        .args(["-sSfL", "--max-time", "30", "-o"])
        .arg(&fetched)
        .arg("https://example.com/")
        .status()
        .map_err(|e| e.to_string())?;
    let result = json!({
        "writtenByApp": quarantined(&written),
        "fetchedByCurl": if status.success() { json!(quarantined(&fetched)) } else { json!(format!("curl failed: {status}")) },
    });
    let _ = std::fs::remove_dir_all(&dir);
    Ok(result)
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;

    let pay = apple_pay(ctx, &server)?;
    checks.check(
        "Apple Pay: canMakePayments recorded with and without a user script (row 10)",
        pay["localhostNoScripts"].is_object() && pay["localhostWithUserScript"].is_object(),
        pay.clone(),
    );

    let sb = safe_browsing(ctx)?;
    checks.check(
        "Safe Browsing: the phishing test page shows a warning (row 17)",
        sb["warningViews"].as_array().is_some_and(|w| !w.is_empty()),
        sb.clone(),
    );

    let archive = web_archive(ctx, &server)?;
    checks.check(
        ".webarchive: saved and shown again with its text and style (row 18)",
        archive["shown"]["text"] == "archived page text"
            && archive["shown"]["color"] == "rgb(1, 2, 3)",
        archive.clone(),
    );

    let q = quarantine()?;
    checks.check(
        "downloads by the app carry no quarantine flag (row 19)",
        q["writtenByApp"].is_null(),
        q.clone(),
    );

    Ok(checks.finish(
        json!({ "applePay": pay, "safeBrowsing": sb, "webArchive": archive, "quarantine": q }),
    ))
}
