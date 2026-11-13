//! Experiment 12 (16 §5 row 12, M0-T08): agent tabs in an off-screen window
//! that is ordered in but never key (13 §7.3). Do pages behave as visible,
//! do requestAnimationFrame and timers run, and do synthesised clicks and
//! typing arrive as trusted input (13 §8)? A hidden tab view in the same
//! window is measured for contrast.

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

const FORM: &str = r#"<!doctype html><meta charset="utf-8"><title>Agent fixture</title>
<style>body{margin:0;font:16px system-ui}#f{position:absolute;left:100px;top:100px;width:300px;height:30px}</style>
<input id="f" aria-label="Field">
<script>
window.__events = [];
for (const type of ["click", "keydown", "input"]) {
  addEventListener(type, (e) => __events.push({ type, trusted: e.isTrusted, key: e.key ?? null }), true);
}
</script>"#;

fn fixture(path: &str) -> Option<String> {
    (path == "/form").then(|| FORM.to_owned())
}

/// Visibility, rAF frames and 50 ms timer ticks over one second.
const LIVENESS: &str = "
  let frames = 0, ticks = 0;
  const start = performance.now();
  const raf = () => { frames++; if (performance.now() - start < 1000) requestAnimationFrame(raf); };
  requestAnimationFrame(raf);
  const timer = setInterval(() => ticks++, 50);
  await new Promise((r) => setTimeout(r, 1000));
  clearInterval(timer);
  return { visibilityState: document.visibilityState, hidden: document.hidden,
           hasFocus: document.hasFocus(), rafFrames: frames, timerTicks: ticks };";

fn eval(ctx: &Ctx, tab: usize, body: &'static str) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, body))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered")?
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    let tab = ctx.tab;
    let window = ctx.window()?;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    macos::clear_navigation(tab);
    let url = server.url("/form");
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;

    // Placement A: on screen, transparent, ignoring the mouse, floating.
    let w = window.clone();
    let transparent = on_main(&ctx.app, move || macos::window_state(&w))??;
    let visible = eval(ctx, tab, LIVENESS)?;
    liveness_checks(&mut checks, "transparent window", &visible);

    // Click the field's centre, then type into it.
    let page_rect = ctx.shell().reports()?.1.ok_or("no page rectangle")?;
    let (x, y) = (page_rect.x + 250.0, page_rect.y + 115.0);
    let w = window.clone();
    let delivery = on_main(&ctx.app, move || {
        let d = macos::click(&w, x, y)?;
        macos::focus_tab(&w, tab)?;
        Ok::<_, String>(d)
    })??;
    let w = window.clone();
    on_main(&ctx.app, move || macos::type_keys(&w, tab, "mado"))??;
    let typed = eval(
        ctx,
        tab,
        "await new Promise((r) => setTimeout(r, 300));
         return { value: document.querySelector('#f').value,
                  active: document.activeElement?.id ?? null, events: window.__events };",
    )?;
    let events = typed["events"].as_array().cloned().unwrap_or_default();
    let all_trusted = !events.is_empty() && events.iter().all(|e| e["trusted"] == true);
    checks.check(
        "click focuses the field",
        typed["active"] == "f",
        json!({ "delivery": delivery, "active": typed["active"] }),
    );
    checks.check(
        "typed keys reach the field",
        typed["value"] == "mado",
        typed["value"].clone(),
    );
    checks.check(
        "every click, keydown and input event is trusted",
        all_trusted,
        json!(events),
    );

    // Contrast: a hidden tab view in the same window.
    let w = window.clone();
    let hidden_tab = on_main(&ctx.app, move || macos::add_tab(&w, true))??;
    let url = server.url("/form");
    macos::clear_navigation(hidden_tab);
    on_main(&ctx.app, move || macos::navigate(hidden_tab, &url))??;
    macos::wait_navigation(hidden_tab, STEP_TIMEOUT)?;
    let hidden = eval(ctx, hidden_tab, LIVENESS)?;

    // Placement B, 13 §7.3 as written: opaque, entirely off every screen.
    let w = window.clone();
    let off_screen = on_main(&ctx.app, move || {
        macos::opaque_window_at(&w, -20000.0, -20000.0)?;
        macos::window_state(&w)
    })??;
    // Give the window server a moment to update the occlusion state.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let w = window.clone();
    let off_screen_later = on_main(&ctx.app, move || macos::window_state(&w))??;
    checks.check(
        "13 §7.3 placement: the window is entirely off every screen and not key",
        off_screen["onAnyScreen"] == false && off_screen["isKey"] == false,
        off_screen.clone(),
    );
    let off = eval(ctx, tab, LIVENESS)?;
    liveness_checks(&mut checks, "13 §7.3 off-screen window", &off);

    Ok(checks.finish(json!({
        "transparentWindow": { "window": transparent, "page": visible },
        "offScreenWindow": { "window": off_screen_later, "page": off },
        "hiddenTabViewForContrast": hidden,
        "typing": typed,
    })))
}

fn liveness_checks(checks: &mut Checks, placement: &str, page: &Value) {
    checks.check(
        &format!("{placement}: page reports visibilityState visible"),
        page["visibilityState"] == "visible",
        page.clone(),
    );
    checks.check(
        &format!("{placement}: requestAnimationFrame runs (at least 30 frames in 1 s)"),
        page["rafFrames"].as_u64().is_some_and(|n| n >= 30),
        page["rafFrames"].clone(),
    );
    checks.check(
        &format!("{placement}: 50 ms timers run unthrottled (at least 15 ticks in 1 s)"),
        page["timerTicks"].as_u64().is_some_and(|n| n >= 15),
        page["timerTicks"].clone(),
    );
}
