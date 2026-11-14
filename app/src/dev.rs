//! The platform test page of development builds (roadmap M1-T10): each
//! `mado-platform` function behind a button, for checks that need a person
//! (Touch ID, system prompts, notifications). Compiled only into debug
//! builds and callable only from the `dev-platform` window.

use mado_platform::permissions::OsPermission;
use mado_platform::{Paths, auth, default_browser, dock, keychain, login, notify, permissions};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "dev-platform";
const SERVICE: &str = "app.mado.devtest";

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return w.set_focus();
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("dev-platform.html".into()))
        .title("Platform test page")
        .inner_size(720.0, 760.0)
        .build()?;
    Ok(())
}

fn permission(name: &str) -> Result<OsPermission, String> {
    Ok(match name {
        "camera" => OsPermission::Camera,
        "microphone" => OsPermission::Microphone,
        "location" => OsPermission::Location,
        "screen" => OsPermission::Screen,
        "speech" => OsPermission::Speech,
        other => return Err(format!("no permission {other}")),
    })
}

fn run(app: &AppHandle, action: &str, arg: &str) -> Result<Value, String> {
    let e = |e: mado_platform::PlatformError| e.to_string();
    Ok(match action {
        "paths" => {
            let p = Paths::system().map_err(|e| e.to_string())?;
            json!({ "root": p.root(), "appDb": p.app_db(), "logs": p.logs(), "bundled": mado_platform::bundled() })
        }
        "keychain.set" => json!(keychain::set(SERVICE, "test", arg.as_bytes()).map_err(e)?),
        "keychain.get" => json!(
            keychain::get(SERVICE, "test")
                .map_err(e)?
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        ),
        "keychain.delete" => json!(keychain::delete(SERVICE, "test").map_err(e)?),
        "auth.available" => json!(auth::available()),
        "auth.authenticate" => {
            json!({ "token": auth::authenticate("test the platform page").map_err(e)? })
        }
        "notify.request" => json!(notify::request_permission().map_err(e)?),
        "notify.post" => {
            let app = app.clone();
            notify::on_click(move |id| {
                let _ = app.emit_to(LABEL, "dev://notification-clicked", id);
            })
            .map_err(e)?;
            json!(notify::post("dev-test", "Mado", arg).map_err(e)?)
        }
        "permission.status" => json!(permissions::status(permission(arg)?).map_err(e)?),
        "permission.request" => json!(permissions::request(permission(arg)?).map_err(e)?),
        "login.enabled" => json!(login::enabled().map_err(e)?),
        "login.set" => json!(login::set(arg == "on").map_err(e)?),
        "defaultBrowser.is" => json!(default_browser::is_default().map_err(e)?),
        "defaultBrowser.make" => json!(default_browser::make_default().map_err(e)?),
        "dock.badge" => {
            let text = arg.to_owned();
            app.run_on_main_thread(move || {
                dock::set_badge((!text.is_empty()).then_some(text.as_str()))
            })
            .map_err(|e| e.to_string())?;
            Value::Null
        }
        other => return Err(format!("no action {other}")),
    })
}

#[tauri::command]
pub async fn mado_dev_platform(
    webview: tauri::Webview,
    action: String,
    arg: String,
) -> Result<Value, String> {
    if webview.label() != LABEL {
        return Err("only the platform test page may call this".into());
    }
    let app = webview.app_handle().clone();
    // Several functions block until the user answers a prompt.
    tauri::async_runtime::spawn_blocking(move || run(&app, &action, &arg))
        .await
        .map_err(|e| e.to_string())?
}
