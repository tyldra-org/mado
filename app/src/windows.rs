//! Browser windows on the main thread (spec 01 §5.1, 02 `WindowCommand`):
//! a Tauri window holding the UI webview (`ui-<WindowId>`), filling it, and
//! the overlay webview (`overlay-<WindowId>`) above, hidden until used.
//! Tab views go between them from M2.

use std::sync::{Arc, OnceLock};

use mado_core::{CoreHandle, Input};
use mado_engine::window::{overlay_label, ui_label};
use mado_engine::{WindowCommand, WindowEngine, WindowEvent};
use mado_types::WindowId;
use mado_types::state::{Bounds, WindowStateKind};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// The Tauri window label of a browser window.
pub fn window_label(window: WindowId) -> String {
    format!("window-{window}")
}

/// The browser window a Tauri window label names.
pub fn window_of(label: &str) -> Option<WindowId> {
    label.strip_prefix("window-")?.parse().ok()
}

/// Carries out window commands from the core on the main thread and reports
/// back through the core's input.
pub struct TauriWindows {
    app: AppHandle,
    core: Arc<OnceLock<CoreHandle>>,
}

impl TauriWindows {
    pub fn new(app: AppHandle, core: Arc<OnceLock<CoreHandle>>) -> Self {
        Self { app, core }
    }
}

impl WindowEngine for TauriWindows {
    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand) {
        let app = self.app.clone();
        let core = Arc::clone(&self.core);
        let scheduled = self.app.run_on_main_thread(move || {
            if let Err(e) = apply(&app, &core, window, cmd) {
                tracing::error!(%window, error = %e, "a window command failed");
            }
        });
        if let Err(e) = scheduled {
            tracing::error!(error = %e, "the main thread is gone");
        }
    }
}

fn apply(
    app: &AppHandle,
    core: &OnceLock<CoreHandle>,
    window: WindowId,
    cmd: WindowCommand,
) -> tauri::Result<()> {
    let label = window_label(window);
    match cmd {
        WindowCommand::Create {
            window: id, bounds, ..
        } => {
            create(app, id, &bounds)?;
            if let Some(core) = core.get() {
                core.send(Input::Window(WindowEvent::Created { window: id }));
            }
        }
        cmd => {
            let Some(w) = app.get_window(&label) else {
                return Ok(());
            };
            match cmd {
                WindowCommand::Close => w.destroy()?,
                WindowCommand::Focus => {
                    app.show()?;
                    w.unminimize()?;
                    w.set_focus()?;
                }
                WindowCommand::SetBounds(b) => {
                    w.set_position(LogicalPosition::new(b.x, b.y))?;
                    w.set_size(LogicalSize::new(b.width, b.height))?;
                }
                WindowCommand::SetState(state) => match state {
                    WindowStateKind::Normal => {
                        w.set_fullscreen(false)?;
                        w.unmaximize()?;
                        w.unminimize()?;
                    }
                    WindowStateKind::Maximized => w.maximize()?,
                    WindowStateKind::Fullscreen => w.set_fullscreen(true)?,
                    WindowStateKind::Minimized => w.minimize()?,
                },
                WindowCommand::ShowOverlay { rect } => {
                    if let Some(overlay) = app.get_webview(&overlay_label(window)) {
                        overlay.set_position(LogicalPosition::new(rect.x, rect.y))?;
                        overlay.set_size(LogicalSize::new(rect.width, rect.height))?;
                        overlay.show()?;
                    }
                }
                WindowCommand::HideOverlay => {
                    if let Some(overlay) = app.get_webview(&overlay_label(window)) {
                        overlay.hide()?;
                    }
                }
                WindowCommand::SetTitle(title) => w.set_title(&title)?,
                WindowCommand::Create { .. } => {}
            }
        }
    }
    Ok(())
}

const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);

fn create(app: &AppHandle, id: WindowId, bounds: &Bounds) -> tauri::Result<()> {
    let (width, height) = if bounds.width > 0.0 {
        (bounds.width, bounds.height)
    } else {
        DEFAULT_SIZE
    };
    let mut builder = WindowBuilder::new(app, window_label(id))
        .title("Mado")
        .inner_size(width, height)
        .min_inner_size(500.0, 300.0);
    if bounds.width > 0.0 {
        builder = builder.position(bounds.x, bounds.y);
    }
    let window = builder.build()?;
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    window.add_child(
        WebviewBuilder::new(ui_label(id), WebviewUrl::App("window.html".into())).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        size,
    )?;
    let overlay = window.add_child(
        WebviewBuilder::new(overlay_label(id), WebviewUrl::App("overlay.html".into()))
            .transparent(true),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(1.0, 1.0),
    )?;
    overlay.hide()?;
    window.set_focus()?;
    Ok(())
}
