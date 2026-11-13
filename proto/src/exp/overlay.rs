//! Experiment 1 (16 §5 row 1, M0-T02): a native tab view between the Tauri UI
//! webview and an overlay webview. Checks the view stack, where clicks land
//! with the overlay shown and hidden, whether Tauri exposes the overlay's
//! native view, and whether the tab view stays aligned through resizes.

use std::sync::mpsc;

use serde_json::{Value, json};
use tauri::{LogicalPosition, LogicalSize, Manager};

use super::Checks;
use crate::macos;
use crate::shell::{Ctx, OVERLAY_LABEL, Rect, STEP_TIMEOUT, on_main};

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>Layers fixture</title>
<style>html,body{margin:0;height:100%;background:#d9ead3;font:16px system-ui}</style>
<p>Tab view fixture page.</p>
<script>
window.__clicks = [];
addEventListener("click", (e) => __clicks.push({ x: e.clientX, y: e.clientY, trusted: e.isTrusted }), true);
</script>"#;

/// How long a click may take to reach a page before it counts as missed.
const CLICK_WAIT_MS: u32 = 1500;
/// Frames closer than this, in points, count as aligned.
const TOLERANCE: f64 = 0.5;

/// Script that resolves with the clicks a page receives from now on, waiting
/// up to `CLICK_WAIT_MS` for the first one.
fn next_clicks() -> String {
    format!(
        "const n = window.__clicks.length, start = performance.now();
         while (window.__clicks.length <= n && performance.now() - start < {CLICK_WAIT_MS})
           await new Promise((r) => setTimeout(r, 16));
         return window.__clicks.slice(n);"
    )
}

type Pending = mpsc::Receiver<Result<Value, String>>;

fn received(rx: &Pending) -> Result<Value, String> {
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered".to_owned())?
}

/// Clicks a CSS point and reports which layer AppKit's hit test names and
/// which pages saw the click.
fn click(ctx: &Ctx, x: f64, y: f64) -> Result<Value, String> {
    let window = ctx.window()?;
    let tab = ctx.tab;
    let (hit, page, overlay, delivery) = on_main(&ctx.app, move || {
        let hit = macos::hit_test(&window, x, y)?;
        let page = macos::eval_tab(tab, &next_clicks())?;
        let overlay = macos::eval_overlay(&next_clicks())?;
        let delivery = macos::click(&window, x, y)?;
        Ok::<_, String>((hit, page, overlay, delivery))
    })??;
    let page = received(&page)?;
    let overlay = received(&overlay)?;
    Ok(json!({
        "at": [x, y],
        "hitTest": hit,
        "delivery": delivery,
        "page": page,
        "overlay": overlay,
    }))
}

fn count(clicks: &Value, layer: &str) -> usize {
    clicks[layer].as_array().map_or(0, Vec::len)
}

fn close(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= TOLERANCE)
}

fn frame(v: &Value) -> [f64; 4] {
    let n = |i: usize| v[i].as_f64().unwrap_or(f64::NAN);
    [n(0), n(1), n(2), n(3)]
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let window = ctx.window()?;
    let overlay = ctx
        .app
        .get_webview(OVERLAY_LABEL)
        .ok_or("no overlay webview")?;

    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    macos::clear_navigation(ctx.tab);
    let tab = ctx.tab;
    on_main(&ctx.app, move || {
        macos::load_html(tab, PAGE, "https://layers.mado.test/")
    })??;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;

    // Layer order.
    let w = window.clone();
    let stack = on_main(&ctx.app, move || macos::view_stack(&w))??;
    checks.check(
        "bottom to top: UI webview, tab view, overlay",
        stack == ["ui", "tab:0", "overlay"],
        json!(stack),
    );

    // Does Tauri expose the overlay's native view?
    let (tx, rx) = mpsc::channel();
    overlay
        .with_webview(move |platform| {
            let _ = tx.send(platform.inner() as usize);
        })
        .map_err(|e| e.to_string())?;
    let from_tauri = rx
        .recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "with_webview never ran")?;
    let registered = on_main(&ctx.app, macos::overlay_address)??;
    checks.check(
        "Webview::with_webview returns the overlay's WKWebView",
        from_tauri == registered && from_tauri != 0,
        json!({ "withWebview": format!("{from_tauri:#x}"), "topmostSubview": format!("{registered:#x}") }),
    );
    let ready = on_main(&ctx.app, || {
        macos::eval_overlay("return Array.isArray(window.__clicks);")
    })??;
    let ready = received(&ready)?;
    checks.check(
        "script runs in the overlay through its native view",
        ready == json!(true),
        ready,
    );

    // The overlay's rectangle: 300 x 200 CSS pixels inside the page area.
    let (_, page) = ctx.shell().reports()?;
    let page = page.ok_or("no page rectangle yet")?;
    let r = Rect {
        x: page.x + 100.0,
        y: page.y + 100.0,
        width: 300.0,
        height: 200.0,
    };
    let inside = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let outside = (r.x + r.width + 100.0, r.y + r.height / 2.0);

    let hidden_click = click(ctx, inside.0, inside.1)?;
    checks.check(
        "overlay hidden: a click at its spot reaches the page",
        hidden_click["hitTest"] == "tab:0"
            && count(&hidden_click, "page") == 1
            && count(&hidden_click, "overlay") == 0,
        hidden_click.clone(),
    );

    // Show the overlay through Tauri's own API. Tauri positions child
    // webviews from the content view's top, which starts above the UI's CSS
    // viewport by the safe-area inset.
    let w = window.clone();
    let safe_top = on_main(&ctx.app, move || macos::ui_safe_top(&w))??;
    overlay
        .set_position(LogicalPosition::new(r.x, r.y + safe_top))
        .and_then(|()| overlay.set_size(LogicalSize::new(r.width, r.height)))
        .and_then(|()| overlay.show())
        .map_err(|e| e.to_string())?;
    let w = window.clone();
    let shown = on_main(&ctx.app, move || {
        Ok::<_, String>((macos::frames(&w, tab)?, macos::expected_frame(&w, r)?))
    })??;
    checks.check(
        "overlay shown only over its rectangle",
        shown.0["overlayHidden"] == false && close(&frame(&shown.0["overlay"]), &shown.1),
        json!({ "frames": shown.0, "expectedOverlay": shown.1, "safeTop": safe_top }),
    );

    let on_overlay = click(ctx, inside.0, inside.1)?;
    checks.check(
        "overlay shown: a click inside it reaches the overlay, not the page",
        on_overlay["hitTest"] == "overlay"
            && count(&on_overlay, "overlay") == 1
            && count(&on_overlay, "page") == 0,
        on_overlay.clone(),
    );
    let beside = click(ctx, outside.0, outside.1)?;
    checks.check(
        "overlay shown: a click beside it reaches the page",
        beside["hitTest"] == "tab:0"
            && count(&beside, "page") == 1
            && count(&beside, "overlay") == 0,
        beside.clone(),
    );

    overlay.hide().map_err(|e| e.to_string())?;
    let after_hide = click(ctx, inside.0, inside.1)?;
    checks.check(
        "overlay hidden again: its spot reaches the page",
        after_hide["hitTest"] == "tab:0" && count(&after_hide, "page") == 1,
        after_hide.clone(),
    );

    // Resizes: the tab view follows the window before the UI reports the new
    // rectangle (autoresizing), and matches it exactly after.
    let mut resizes = Vec::new();
    for (width, height) in [(1000.0, 700.0), (1400.0, 900.0), (1200.0, 800.0)] {
        let (seen, last) = ctx.shell().reports()?;
        let last = last.ok_or("no page rectangle yet")?;
        let predicted = Rect {
            x: last.x,
            y: last.y,
            width,
            height: height - safe_top - last.y,
        };
        let w = window.clone();
        // Resize natively and measure in the same main-thread turn: what the
        // user sees between the native resize and the UI's report.
        let before = on_main(&ctx.app, move || {
            macos::resize(&w, width, height)?;
            Ok::<_, String>((
                macos::frames(&w, tab)?,
                macos::expected_frame(&w, predicted)?,
            ))
        })??;
        let (_, reported) = ctx.shell().wait_report(seen, STEP_TIMEOUT)?;
        let w = window.clone();
        let after = on_main(&ctx.app, move || {
            Ok::<_, String>((
                macos::frames(&w, tab)?,
                macos::expected_frame(&w, reported)?,
            ))
        })??;
        let aligned_before = close(&frame(&before.0["tab"]), &before.1);
        let aligned_after = close(&frame(&after.0["tab"]), &after.1);
        checks.check(
            &format!("{width} x {height}: tab view aligned before the UI reports"),
            aligned_before,
            json!({ "tab": before.0["tab"], "expected": before.1 }),
        );
        checks.check(
            &format!("{width} x {height}: tab view aligned after the UI reports"),
            aligned_after && reported == predicted,
            json!({ "tab": after.0["tab"], "expected": after.1, "reported": reported }),
        );
        resizes
            .push(json!({ "size": [width, height], "predicted": predicted, "reported": reported }));
    }
    // Leave the window as the screenshot should show it: overlay open.
    overlay.show().map_err(|e| e.to_string())?;

    Ok(checks.finish(json!({ "overlayRect": r, "resizes": resizes })))
}
