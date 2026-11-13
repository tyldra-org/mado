//! Experiment 2 (16 §5 row 2, M0-T03): Chrome Web Store extensions in
//! `WKWebExtensionController` on the self-signed build. Each extension is
//! downloaded from the store, loaded with its Chrome ID as unique
//! identifier, and its popup opened. Core features without an account:
//! uBlock Origin Lite blocks an ad script, Dark Reader darkens a page.
//! Bitwarden and 1Password need accounts for their core features.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};
use crate::{macos, webext};

const EXTENSIONS: [(&str, &str); 4] = [
    ("uBlock Origin Lite", "ddkjiahejlhfcafbddmgiahcphecmpfh"),
    ("Dark Reader", "eimadpbcbfnmbkopoojfekhnkhdbieeh"),
    ("Bitwarden", "nngceckbapebfimnlniiiahkandclblb"),
    ("1Password", "aeblfdkhhhdcdjpifhhbdiojplfjncoa"),
];

const PAGE: &str = r#"<!doctype html><title>Extension fixture</title>
<style>body{background:#fff;color:#000;font:16px system-ui}</style>
<p>Fixture text for Dark Reader.</p>
<script>
window.__ads = "pending";
const s = document.createElement("script");
s.src = "https://securepubads.g.doubleclick.net/tag/js/gpt.js";
s.onload = () => { window.__ads = "loaded"; };
s.onerror = () => { window.__ads = "blocked"; };
document.head.appendChild(s);
</script>"#;

fn fixture(path: &str) -> Option<String> {
    (path == "/page").then(|| PAGE.to_owned())
}

/// Downloads the extension's CRX from the Chrome Web Store (cached a day)
/// and unpacks it.
fn fetch(id: &str) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join("mado-proto-extensions");
    let crx = root.join(format!("{id}.crx"));
    let dir = root.join(id);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let is_crx = std::fs::read(&crx).is_ok_and(|b| b.starts_with(b"Cr24"));
    let fresh = is_crx
        && std::fs::metadata(&crx)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(24 * 3600));
    if !fresh {
        // The store answers with an empty body when `prodversion` is older
        // than the extension's minimum Chrome version (Bitwarden and
        // 1Password refused 131), so ask as a current Chrome.
        let url = format!(
            "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=157.0.0.0&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
        );
        let status = std::process::Command::new("/usr/bin/curl")
            .args(["-sSfL", "--max-time", "60", "-o"])
            .arg(&crx)
            .arg(&url)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("{id}: download failed ({status})"));
        }
    }
    let bytes = std::fs::read(&crx).map_err(|e| e.to_string())?;
    // CRX3: "Cr24", version, header length, header, then the zip.
    if bytes.get(..4) != Some(b"Cr24") || bytes.len() < 12 {
        return Err(format!("{id}: not a CRX file"));
    }
    let header = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let zip_bytes = bytes.get(12 + header..).ok_or("truncated CRX")?;
    let zip = root.join(format!("{id}.zip"));
    std::fs::write(&zip, zip_bytes).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    let status = std::process::Command::new("/usr/bin/unzip")
        .args(["-q", "-o"])
        .arg(&zip)
        .arg("-d")
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    // unzip exits 1 for warnings (such as Chrome's extra header bytes).
    if status.code().is_some_and(|c| c > 1) {
        return Err(format!("{id}: unzip failed ({status})"));
    }
    Ok(dir)
}

fn manifest_version(dir: &Path) -> Value {
    std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map_or(
            Value::Null,
            |m| json!({ "manifestVersion": m["manifest_version"], "version": m["version"] }),
        )
}

fn wait<T>(rx: &std::sync::mpsc::Receiver<Result<T, String>>) -> Result<T, String> {
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "WebKit never answered".to_owned())?
}

fn eval(ctx: &Ctx, tab: usize, body: &'static str) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, body))??;
    wait(&rx)
}

fn load(ctx: &Ctx, tab: usize, url: String) -> Result<Value, String> {
    macos::clear_navigation(tab);
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)
}

/// The popup's page, once it has loaded (up to 10 s).
fn popup_state(ctx: &Ctx, id: &'static str) -> Result<Value, String> {
    let started = Instant::now();
    loop {
        let rx = on_main(&ctx.app, move || {
            webext::eval_popup(
                id,
                "return { url: location.href, ready: document.readyState, title: document.title,
                          runtimeId: globalThis.chrome?.runtime?.id ?? globalThis.browser?.runtime?.id ?? null,
                          textLength: document.body?.innerText.length ?? 0,
                          elements: document.querySelectorAll('*').length,
                          bodyHtml: (document.body?.innerHTML ?? '').slice(0, 300),
                          visibility: document.visibilityState };",
            )
        })??;
        let state = wait(&rx);
        if let Ok(s) = &state
            && s["ready"] == "complete"
            && s["textLength"].as_u64().is_some_and(|n| n > 0)
        {
            return state;
        }
        if started.elapsed() > Duration::from_secs(20) {
            return state;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let mut report = Vec::new();
    for (name, id) in EXTENSIONS {
        let dir = match fetch(id) {
            Ok(dir) => dir,
            Err(e) => {
                checks.check(&format!("{name}: loads"), false, json!(e));
                continue;
            }
        };
        let rx = on_main(&ctx.app, {
            let dir = dir.clone();
            move || webext::load(&dir, id)
        })??;
        let loaded = wait(&rx);
        let ok = loaded.as_ref().is_ok_and(|l| l["loaded"] == true);
        checks.check(
            &format!("{name}: loads"),
            ok,
            loaded.clone().unwrap_or_else(|e| json!(e)),
        );
        let background = if ok {
            let rx = on_main(&ctx.app, move || webext::load_background(id))??;
            match rx.recv_timeout(STEP_TIMEOUT) {
                Ok(Ok(())) => json!("started"),
                Ok(Err(e)) => json!(e),
                Err(_) => json!("no answer"),
            }
        } else {
            Value::Null
        };
        let popup = if ok {
            let w = ctx.window()?;
            let opened = on_main(&ctx.app, move || webext::open_popup(&w, id))?;
            let page = opened
                .as_ref()
                .ok()
                .filter(|o| o["popupWebView"] == true)
                .map(|_| popup_state(ctx, id));
            let page = page.map(|p| p.unwrap_or_else(|e| json!(e)));
            let shows = page
                .as_ref()
                .is_some_and(|p| p["textLength"].as_u64().is_some_and(|n| n > 0));
            checks.check(
                &format!("{name}: popup shows"),
                shows,
                json!({ "action": opened.clone().unwrap_or_else(|e| json!(e)), "page": page }),
            );
            if let Some(p) = &page {
                checks.check(
                    &format!("{name}: chrome.runtime.id is the Chrome ID"),
                    p["runtimeId"] == id,
                    p["runtimeId"].clone(),
                );
            }
            json!({ "action": opened.unwrap_or_else(|e| json!(e)), "page": page })
        } else {
            Value::Null
        };
        report.push(json!({ "name": name, "id": id, "manifest": manifest_version(&dir), "load": loaded.unwrap_or_else(|e| json!(e)), "background": background, "popup": popup }));
    }

    // Core features without an account, in a tab that runs the extensions,
    // against a control tab that does not.
    let window = ctx.window()?;
    let w = window.clone();
    let with = on_main(&ctx.app, move || webext::add_tab(&w))??;
    let w = window.clone();
    let without = on_main(&ctx.app, move || macos::add_tab_with_store(&w, None))??;
    let probe = "await new Promise((r) => setTimeout(r, 4000));
                 return { ads: window.__ads, background: getComputedStyle(document.body).backgroundColor };";
    let control = load(ctx, without, server.url("/page")).and_then(|_| eval(ctx, without, probe));
    let extended = load(ctx, with, server.url("/page")).and_then(|_| eval(ctx, with, probe));
    let (control, extended) = (
        control.unwrap_or_else(|e| json!(e)),
        extended.unwrap_or_else(|e| json!(e)),
    );
    checks.check(
        "uBlock Origin Lite: the ad script is blocked with it and loads without it",
        extended["ads"] == "blocked" && control["ads"] == "loaded",
        json!({ "with": extended["ads"], "without": control["ads"] }),
    );
    checks.check(
        "Dark Reader: the white page turns dark",
        control["background"] == "rgb(255, 255, 255)"
            && extended["background"] != control["background"],
        json!({ "with": extended["background"], "without": control["background"] }),
    );
    Ok(checks.finish(
        json!({ "extensions": report, "coreFeatures": { "with": extended, "without": control } }),
    ))
}
