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
