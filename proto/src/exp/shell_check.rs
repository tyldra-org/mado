//! M0-T01: typing a URL into the UI's address field navigates the tab view.

use serde_json::{Value, json};
use tauri::Manager;

use super::Checks;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, UI_LABEL, normalize_url, on_main};

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let typed = ctx.arg.clone().unwrap_or_else(|| "example.org".into());
    // Let the first page finish, so the typed URL is a second navigation.
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    macos::clear_navigation(ctx.tab);
    let ui = ctx.app.get_webview(UI_LABEL).ok_or("no UI webview")?;
    let literal = serde_json::to_string(&typed).map_err(|e| e.to_string())?;
    ui.eval(format!("window.madoProto.typeAndGo({literal})"))
        .map_err(|e| e.to_string())?;
    let loaded = macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let window = ctx.window()?;
    let views = on_main(&ctx.app, move || macos::describe_views(&window))??;

    let mut checks = Checks::new();
    let expected = normalize_url(&typed)?;
    let url = loaded["url"].as_str().unwrap_or_default();
    checks.check(
        "the tab view loads the typed address",
        url.trim_end_matches('/') == expected.trim_end_matches('/'),
        json!(url),
    );
    Ok(checks.finish(json!({ "typed": typed, "views": views })))
}
