//! Experiment 15 (07 §2, §4): speech recognition, screen sharing, the
//! notifications shim and the geolocation bridge on WebKit, without a single
//! OS prompt. The recording UI delegate denies every request WebKit makes,
//! so the run learns which delegate path each API takes and what the page
//! sees on denial. The shims are the page-world scripts 07 describes,
//! talking to the run through an outbox the run polls (Mado's bridge uses a
//! script message handler, the same channel Tauri's IPC already uses). Their
//! native ends are real: notifications go through UserNotifications with
//! provisional authorization (quiet, no prompt) and are removed again;
//! positions come from a fixture provider, since CoreLocation would prompt.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};
use crate::{macos, perm};

fn fixture(path: &str) -> Option<String> {
    (path == "/permissions").then(|| {
        "<!doctype html><title>M0 permissions</title><p>Permissions fixture</p>".to_owned()
    })
}

fn eval(ctx: &Ctx, tab: usize, body: String) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, &body))??;
    rx.recv_timeout(Duration::from_secs(40))
        .map_err(|_| "script never answered")?
}

/// The page's own APIs, each answered by the recording delegate (deny).
const NATIVE: &str = r#"
const out = { secure: isSecureContext, activation: navigator.userActivation ? navigator.userActivation.isActive : null };
const timeout = (p, ms) => Promise.race([p, new Promise((r) => setTimeout(() => r('timeout'), ms))]);
out.getDisplayMedia = typeof navigator.mediaDevices?.getDisplayMedia;
if (out.getDisplayMedia === 'function') {
  out.display = await timeout(navigator.mediaDevices.getDisplayMedia({ video: true })
    .then((s) => { s.getTracks().forEach((t) => t.stop()); return 'granted'; }, (e) => e.name), 8000);
}
const R = window.SpeechRecognition || window.webkitSpeechRecognition;
out.speechApi = typeof R;
if (R && __SPEECH__) {
  out.speech = await timeout(new Promise((res) => {
    const r = new R();
    r.onerror = (e) => res('error ' + e.error);
    r.onstart = () => { r.abort(); res('started'); };
    try { r.start(); } catch (e) { res('threw ' + e.name); }
  }), 8000);
}
out.notificationApi = typeof Notification;
if (typeof Notification === 'function') {
  out.notificationBefore = Notification.permission;
  out.notification = await timeout(Notification.requestPermission(), 8000);
}
out.geolocationApi = typeof navigator.geolocation?.getCurrentPosition;
if (out.geolocationApi === 'function') {
  out.geolocation = await timeout(new Promise((res) => navigator.geolocation.getCurrentPosition(
    () => res('position'), (e) => res('error ' + e.code), { timeout: 5000 })), 8000);
}
return out;"#;

/// The `notifications` and `geolocation` page scripts of 07 §2, with an
/// outbox standing in for the bridge.
const SHIMS: &str = r#"
const pending = new Map();
let next = 1;
const outbox = (window.__madoOutbox = []);
const post = (kind, data) => new Promise((resolve, reject) => {
  const id = next++;
  pending.set(id, { resolve, reject });
  outbox.push({ id, kind, data });
});
window.__madoAnswer = (id, ok, value) => {
  const p = pending.get(id);
  pending.delete(id);
  if (ok) p.resolve(value); else p.reject(value);
};
let permission = 'default';
class Notification extends EventTarget {
  constructor(title, options = {}) {
    super();
    this.title = String(title);
    this.body = String(options.body ?? '');
    post('notification.show', { title: this.title, body: this.body }).then(
      () => this.dispatchEvent(new Event('show')),
      () => this.dispatchEvent(new Event('error')));
  }
  static get permission() { return permission; }
  static async requestPermission() { permission = await post('notification.request', {}); return permission; }
}
Object.defineProperty(window, 'Notification', { value: Notification, configurable: true, writable: true });
const geolocation = {
  getCurrentPosition(ok, err) {
    post('geolocation.current', {}).then((coords) => ok({ coords, timestamp: Date.now() }), (e) => err && err(e));
  },
  watchPosition() { return 0; },
  clearWatch() {},
};
Object.defineProperty(navigator, 'geolocation', { value: geolocation, configurable: true });
const results = (window.__madoResults = {});
Notification.requestPermission().then((p) => {
  results.permission = p;
  const n = new Notification('Mado M0', { body: 'Experiment 15, removed right away' });
  n.addEventListener('show', () => { results.shown = true; });
  n.addEventListener('error', () => { results.shown = false; });
});
navigator.geolocation.getCurrentPosition(
  (p) => { results.position = { latitude: p.coords.latitude, longitude: p.coords.longitude, accuracy: p.coords.accuracy }; },
  (e) => { results.positionError = e; });
return true;"#;

/// Set to run the speech recognition step, which shows OS prompts.
const SUPERVISED: &str = "MADO_PROTO_SUPERVISED";

const NOTIFICATION_ID: &str = "app.mado.proto.m0.experiment-15";

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let window = ctx.window()?;
    let tab = on_main(&ctx.app, move || -> Result<usize, String> {
        let tab = macos::add_tab(&window, false)?;
        perm::watch(tab)?;
        Ok(tab)
    })??;
    macos::clear_navigation(tab);
    let url = server.url("/permissions");
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;

    let os_before = on_main(&ctx.app, perm::os_states)??;
    // WebKit asks macOS for speech recognition (and the microphone) before
    // it asks the delegate, so starting recognition shows an OS prompt; it
    // runs only when someone is at the Mac to answer it.
    let supervised = std::env::var_os(SUPERVISED).is_some();
    let page = eval(
        ctx,
        tab,
        NATIVE.replace("__SPEECH__", &supervised.to_string()),
    )?;
    let asked = perm::ASKED.lock().map_err(|e| e.to_string())?.clone();
    let asked_for = |part: &str| {
        asked
            .iter()
            .any(|a| a["selector"].as_str().is_some_and(|s| s.contains(part)))
    };

    checks.check(
        "screen: getDisplayMedia asks the private display-capture delegate; a denial reaches the page as NotAllowedError, with no OS prompt",
        asked_for("requestDisplayCapturePermission") && page["display"] == "NotAllowedError",
        json!({ "page": page["display"], "api": page["getDisplayMedia"] }),
    );
    if supervised {
        checks.check(
            "speech: SpeechRecognition reaches the private speech delegate after the OS prompts; a denial reaches the page as not-allowed",
            asked_for("requestSpeechRecognitionPermission") && page["speech"] == "error not-allowed",
            json!({ "page": page["speech"], "api": page["speechApi"] }),
        );
    }
    checks.check(
        "location: WebKit's own geolocation asks the public delegate (07 \"macOS 27 delegate\"); a denial reaches the page as error 1",
        asked_for("requestGeolocationPermission") && page["geolocation"] == "error 1",
        json!({ "page": page["geolocation"], "api": page["geolocationApi"] }),
    );

    // The shims, served by the run as the core would serve the bridge.
    let notification_before = perm::notification_status();
    eval(ctx, tab, SHIMS.to_owned())?;
    let fixture_position = json!({ "latitude": 35.6812, "longitude": 139.7671, "accuracy": 10.0 });
    let mut bridged = Vec::new();
    let mut provisional = Value::Null;
    let mut posted = Value::Null;
    let started = Instant::now();
    let results = loop {
        let outbox = eval(ctx, tab, "return window.__madoOutbox.splice(0);".to_owned())?;
        for message in outbox.as_array().cloned().unwrap_or_default() {
            let id = message["id"].as_u64().unwrap_or(0);
            let kind = message["kind"].as_str().unwrap_or("").to_owned();
            let (ok, value) = match kind.as_str() {
                "notification.request" => {
                    provisional =
                        perm::request_provisional().unwrap_or_else(|e| json!({ "error": e }));
                    let status = perm::notification_status().unwrap_or(-1);
                    (
                        true,
                        json!(if matches!(status, 2 | 3) {
                            "granted"
                        } else {
                            "denied"
                        }),
                    )
                }
                "notification.show" => {
                    let title = message["data"]["title"].as_str().unwrap_or("").to_owned();
                    let body = message["data"]["body"].as_str().unwrap_or("").to_owned();
                    let error = perm::post(NOTIFICATION_ID, &title, &body).unwrap_or_else(Some);
                    posted = json!({ "error": error });
                    (error.is_none(), json!(error))
                }
                "geolocation.current" => (true, fixture_position.clone()),
                _ => (false, json!("unknown message")),
            };
            bridged.push(json!({ "kind": kind, "ok": ok }));
            eval(
                ctx,
                tab,
                format!("window.__madoAnswer({id}, {ok}, {value}); return true;"),
            )?;
        }
        let results = eval(ctx, tab, "return window.__madoResults;".to_owned())?;
        let done = results.get("shown").is_some()
            && (results.get("position").is_some() || results.get("positionError").is_some());
        if done || started.elapsed() > Duration::from_secs(15) {
            break results;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    std::thread::sleep(Duration::from_millis(500));
    let delivered = perm::delivered().unwrap_or_default();
    let was_delivered = delivered.iter().any(|d| d == NOTIFICATION_ID);
    perm::remove(NOTIFICATION_ID)?;
    std::thread::sleep(Duration::from_millis(500));
    let after_remove = perm::delivered().unwrap_or_default();
    let removed = !after_remove.iter().any(|d| d == NOTIFICATION_ID);

    checks.check(
        "notifications shim: requestPermission and new Notification reach the bridge, and the native notification is delivered (quietly) and removed",
        results["permission"] == "granted" && results["shown"] == true && was_delivered && removed,
        json!({
            "page": results, "statusBefore": notification_before.ok(), "provisional": provisional,
            "post": posted, "delivered": was_delivered, "removed": removed,
        }),
    );
    checks.check(
        "geolocation bridge: the page's getCurrentPosition gets the position the core posts back",
        results["position"]["latitude"] == fixture_position["latitude"],
        json!({ "page": results["position"] }),
    );
    let os_after = on_main(&ctx.app, perm::os_states)??;
    checks.check(
        "OS permission states read without prompting (07 §4)",
        os_before.is_object() && os_after.is_object(),
        json!({ "before": os_before, "after": os_after }),
    );
    Ok(
        checks
            .finish(json!({ "asked": asked, "page": page, "bridged": bridged, "shims": results })),
    )
}
