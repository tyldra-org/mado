//! Experiment 8 (16 §5 row 8, M0-T05): does `interactionState` restore a
//! tab's back and forward history, and its scroll position, into a view that
//! never saw the pages, as a slept or relaunched tab would be?

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

/// Fixture pages /a, /b (tall, to scroll) and /c.
fn fixture(path: &str) -> Option<String> {
    let (name, height) = match path {
        "/a" => ("A", 400),
        "/b" => ("B", 5000),
        "/c" => ("C", 400),
        _ => return None,
    };
    Some(format!(
        "<!doctype html><title>{name}</title><body style=\"margin:0;height:{height}px\">Page {name}</body>"
    ))
}

fn navigate(ctx: &Ctx, tab: usize, url: String) -> Result<Value, String> {
    macos::clear_navigation(tab);
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)
}

fn eval(ctx: &Ctx, tab: usize, body: &'static str) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, body))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered")?
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let tab = ctx.tab;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    let server = Fixtures::start(fixture)?;
    let pages = [server.url("/a"), server.url("/b"), server.url("/c")];
    for url in &pages {
        navigate(ctx, tab, url.clone())?;
    }
    // Back to B, scrolled down.
    macos::clear_navigation(tab);
    on_main(&ctx.app, move || macos::go_back(tab))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    let scrolled = eval(ctx, tab, "scrollTo(0, 1200); return scrollY;")?;
    let before = on_main(&ctx.app, move || macos::history(tab))??;

    // Save the state through a file, as session restore will.
    let (bytes, class) = on_main(&ctx.app, move || macos::interaction_state(tab))??;
    let file = std::env::temp_dir().join(format!("mado-proto-state-{}.bin", std::process::id()));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;

    // Sleep the tab: a fresh view takes its place, then the old one goes.
    let window = ctx.window()?;
    let restored = on_main(&ctx.app, move || {
        let new = macos::add_tab(&window, true)?;
        macos::close_tab(tab)?;
        Ok::<_, String>(new)
    })??;
    let saved = std::fs::read(&file).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&file);
    macos::clear_navigation(restored);
    on_main(&ctx.app, move || {
        macos::set_interaction_state(restored, &saved)?;
        macos::show_tab(restored)
    })??;
    let restored_load = macos::wait_navigation(restored, STEP_TIMEOUT);
    let after = on_main(&ctx.app, move || macos::history(restored))??;
    let scroll_after = eval(
        ctx,
        restored,
        // WebKit may restore the scroll position a little after the load.
        "const start = performance.now();
         while (scrollY === 0 && performance.now() - start < 2000)
           await new Promise((r) => setTimeout(r, 50));
         return scrollY;",
    )?;

    checks.check(
        "restored tab shows the same page",
        after["url"] == before["url"],
        json!([before["url"], after["url"]]),
    );
    checks.check(
        "back list survives",
        after["back"] == before["back"],
        json!([before["back"], after["back"]]),
    );
    checks.check(
        "forward list survives",
        after["forward"] == before["forward"],
        json!([before["forward"], after["forward"]]),
    );
    // The restored history works: back reaches A.
    macos::clear_navigation(restored);
    on_main(&ctx.app, move || macos::go_back(restored))??;
    let back = macos::wait_navigation(restored, STEP_TIMEOUT)?;
    checks.check(
        "back from the restored tab reaches the first page",
        back["url"].as_str() == Some(pages[0].as_str()),
        back.clone(),
    );

    Ok(checks.finish(json!({
        "stateClass": class,
        "stateBytes": bytes.len(),
        "restoreLoad": restored_load.map_err(|e| json!(e)).unwrap_or_else(|e| e),
        "before": before,
        "after": after,
        // Not a check: 02 promises history and the current page, not scroll.
        "scrollY": { "saved": scrolled, "restored": scroll_after },
        "serverRequests": server.requests().iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
    })))
}
