//! The UI's only door into the core (spec 01 §4.1): one Tauri command,
//! `mado_invoke`, whose origin comes from the calling webview's label, and
//! the `mado://patch` and `mado://notice` events.

use std::sync::{Arc, OnceLock};

use mado_core::{CoreHandle, UiSink};
use mado_types::command::{Origin, Request, Response};
use mado_types::state::Notice;
use mado_types::{ErrorCode, MadoError, Patch, WindowId};
use tauri::{AppHandle, Emitter, EventTarget};

/// Shared with the Tauri command.
pub struct Shared {
    pub core: Arc<OnceLock<CoreHandle>>,
    /// Set when Mado really quits, so the exit is not prevented.
    pub quitting: Arc<std::sync::atomic::AtomicBool>,
}

/// The origin a webview label carries. Nothing in the payload can claim one.
pub fn origin_of(label: &str) -> Option<Origin> {
    if label == "settings" {
        return Some(Origin::Settings);
    }
    if let Some(id) = label.strip_prefix("ui-") {
        return id
            .parse::<WindowId>()
            .ok()
            .map(|window| Origin::Ui { window });
    }
    if let Some(id) = label.strip_prefix("overlay-") {
        return id
            .parse::<WindowId>()
            .ok()
            .map(|window| Origin::Overlay { window });
    }
    None
}

#[tauri::command]
pub async fn mado_invoke(
    webview: tauri::Webview,
    shared: tauri::State<'_, Shared>,
    request: Request,
) -> Result<Response, ()> {
    let id = request.id.clone();
    let Some(origin) = origin_of(webview.label()) else {
        let error = MadoError::new(ErrorCode::PermissionDenied, "error.invoke.notAllowed");
        return Ok(Response::Err { id, error });
    };
    let Some(core) = shared.core.get() else {
        return Ok(Response::Err {
            id,
            error: MadoError::new(ErrorCode::Internal, "error.coreStopped"),
        });
    };
    let reply = core.command(origin, request.cmd).await;
    Ok(match reply {
        Ok(Ok(value)) => Response::Ok { id, value },
        Ok(Err(error)) => Response::Err { id, error },
        Err(_) => Response::Err {
            id,
            error: MadoError::new(ErrorCode::Internal, "error.coreStopped"),
        },
    })
}

/// Webviews that may hear the core: the UI and overlay of each window and
/// the settings window. Tab views are not Tauri webviews at all.
fn listens(target: &EventTarget) -> bool {
    let label = match target {
        EventTarget::Webview { label } | EventTarget::WebviewWindow { label } => label,
        _ => return false,
    };
    origin_of(label).is_some()
}

pub struct TauriUi {
    pub app: AppHandle,
}

impl UiSink for TauriUi {
    fn open_settings(&mut self) {
        self.open_settings_at("");
    }

    fn open_settings_at(&mut self, section: &str) {
        let app = self.app.clone();
        let section = section.to_owned();
        let _ = self.app.run_on_main_thread(move || {
            if let Err(e) = crate::open_settings_at(&app, &section) {
                tracing::error!(error = %e, "the settings window did not open");
            }
        });
    }

    fn window_action(&mut self, window: mado_types::WindowId, action: &str) {
        let label = mado_engine::window::ui_label(window);
        if let Some(w) =
            tauri::Manager::get_window(&self.app, &crate::windows::window_label(window))
        {
            let _ = w.set_focus();
        }
        if let Err(e) = self.app.emit_to(
            tauri::EventTarget::webview(label),
            "mado://action",
            serde_json::json!({ "action": action }),
        ) {
            tracing::warn!(error = %e, "a menu action did not reach the window");
        }
    }

    fn menu_state(&mut self, state: &std::collections::BTreeMap<String, (bool, bool)>) {
        let (app, state) = (self.app.clone(), state.clone());
        let _ = self
            .app
            .run_on_main_thread(move || crate::menu::apply_state(&app, &state));
    }

    fn shortcuts(&mut self, keys: &std::collections::BTreeMap<String, String>) {
        let (app, keys) = (self.app.clone(), keys.clone());
        let _ = self
            .app
            .run_on_main_thread(move || crate::menu::apply_shortcuts(&app, &keys));
    }

    fn patch(&mut self, patch: &Patch) {
        if let Err(e) = self.app.emit_filter("mado://patch", patch, listens) {
            tracing::error!(error = %e, "a patch was not delivered");
        }
    }

    fn notice(&mut self, notice: &Notice) {
        if let Err(e) = self.app.emit_filter("mado://notice", notice, listens) {
            tracing::error!(error = %e, "a notice was not delivered");
        }
    }

    fn show_notification(&mut self, id: &str, title: &str, body: &str) {
        let (id, title, body) = (id.to_owned(), title.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            if let Err(e) = mado_platform::notify::post(&id, &title, &body) {
                tracing::warn!(error = %e, "a notification was not shown");
            }
        });
    }

    fn open_system_settings(&mut self, pane: &str) {
        let anchor = match pane {
            "camera" => "Privacy_Camera",
            "microphone" => "Privacy_Microphone",
            "location" => "Privacy_LocationServices",
            "screen" => "Privacy_ScreenCapture",
            "speechRecognition" => "Privacy_SpeechRecognition",
            "fullDiskAccess" => "Privacy_AllFiles",
            "softwareUpdate" => {
                std::thread::spawn(|| {
                    let _ = std::process::Command::new("/usr/bin/open")
                        .arg("x-apple.systempreferences:com.apple.Software-Update-Settings.extension")
                        .status();
                });
                return;
            }
            _ => "Privacy",
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
        std::thread::spawn(move || {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(url)
                .status();
        });
    }

    fn copy_text(&mut self, text: &str) {
        let text = text.to_owned();
        let _ = self.app.run_on_main_thread(move || {
            if let Err(e) = mado_platform::files::copy_text(&text) {
                tracing::warn!(error = %e, "the pasteboard refused the text");
            }
        });
    }

    fn launch_app_link(&mut self, url: &str) {
        let url = url.to_owned();
        std::thread::spawn(move || {
            if let Err(e) = mado_platform::applinks::launch(&url) {
                tracing::warn!(error = %e, "an app link did not open");
            }
        });
    }

    fn open_in_safari(&mut self, url: &str) {
        let url = url.to_owned();
        std::thread::spawn(move || {
            if let Err(e) = mado_platform::files::open_in_safari(&url) {
                tracing::warn!(error = %e, "Safari did not open");
            }
        });
    }

    fn suggest(&mut self, window: WindowId, payload: &serde_json::Value) {
        let label = format!("ui-{window}");
        if let Err(e) = self
            .app
            .emit_to(EventTarget::AnyLabel { label }, "mado://suggest", payload)
        {
            tracing::warn!(error = %e, "suggestions were not delivered");
        }
    }

    fn quit(&mut self) {
        if let Some(shared) = tauri::Manager::try_state::<Shared>(&self.app) {
            shared
                .quitting
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.app.exit(0);
    }

    fn open_temporary(&mut self, guest: bool, urls: &[String]) {
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(if guest { "--guest" } else { "--private" })
                .args(urls)
                .spawn()
        });
        if let Err(e) = started {
            tracing::error!(error = %e, "the private window did not start");
        }
    }

    fn profile_changed(&mut self, info: &mado_types::state::ProfileInfo) {
        let initial: String = info
            .name
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        let _ = self.app.run_on_main_thread(move || {
            mado_platform::dock::set_badge(Some(&initial));
        });
    }

    fn open_in_profile(&mut self, profile: mado_types::ProfileId, url: &str) {
        // A launch with --profile hands the URL to that profile's process,
        // starting it if needed (spec 01 §1.3).
        if let Err(e) = crate::profile_service::launch(profile, Some(url)) {
            tracing::error!(error = %e, "the other profile did not start");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_labels_carry_an_origin() {
        let w = WindowId::new();
        assert_eq!(
            origin_of(&format!("ui-{w}")),
            Some(Origin::Ui { window: w })
        );
        assert_eq!(
            origin_of(&format!("overlay-{w}")),
            Some(Origin::Overlay { window: w })
        );
        assert_eq!(origin_of("settings"), Some(Origin::Settings));
        for bad in ["picker", "ui-", "ui-not-an-id", "tab-1", "Settings", ""] {
            assert_eq!(origin_of(bad), None, "{bad}");
        }
    }
}
