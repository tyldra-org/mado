//! Browser windows on the main thread (spec 01 §5.1, 02 `WindowCommand`):
//! a Tauri window holding the UI webview (`ui-<WindowId>`), filling it, and
//! the overlay webview (`overlay-<WindowId>`) above, hidden until used.
//! Tab views go between them with the WebKit adapter (M2-T02).

use std::sync::{Arc, Mutex, OnceLock};

use mado_core::{CoreHandle, Input};
use mado_engine::window::{overlay_label, ui_label};
use mado_engine::{
    Callback, Capabilities, ClearDataRequest, Engine, EngineError, EngineEvent, EngineHandle,
    EventSink, PageScriptSet, ProxyConfig, TabCommand, TabSpec, WindowCommand,
};
use mado_types::state::{Bounds, WindowStateKind};
use mado_types::{TabId, WindowId};
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

/// The engine on the main thread: Tauri windows now, WebKit tab views with
/// the adapter (M2-T02). Results go back through the core's input.
pub struct TauriWindows {
    app: AppHandle,
    core: Arc<OnceLock<CoreHandle>>,
    caps: Capabilities,
}

impl TauriWindows {
    /// The core's handle on the engine: each call runs on the main thread.
    pub fn handle(app: AppHandle, core: Arc<OnceLock<CoreHandle>>) -> EngineHandle {
        let sink = EventSink::new({
            let core = Arc::clone(&core);
            move |event| {
                if let Some(core) = core.get() {
                    core.send(Input::Engine(event));
                }
            }
        });
        let engine = Arc::new(Mutex::new(Self {
            app: app.clone(),
            core,
            caps: Capabilities::webkit(),
        }));
        EngineHandle::new(move |op| {
            let engine = Arc::clone(&engine);
            let sink = sink.clone();
            let scheduled = app.run_on_main_thread(move || {
                let mut engine = engine.lock().unwrap_or_else(|e| e.into_inner());
                op.run(&mut *engine, &sink);
            });
            if let Err(e) = scheduled {
                tracing::error!(error = %e, "the main thread is gone");
            }
        })
    }
}

impl Engine for TauriWindows {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn create_tab(&mut self, _spec: TabSpec) -> Result<(), EngineError> {
        Err(EngineError::Unsupported(
            "tab views before the WebKit adapter (M2-T02)",
        ))
    }

    fn destroy_tab(&mut self, _tab: TabId) {}

    fn apply(&mut self, _tab: TabId, cmd: TabCommand) -> Result<(), EngineError> {
        cmd.fail(EngineError::NoTab);
        Err(EngineError::NoTab)
    }

    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand) {
        if let Err(e) = apply(&self.app, &self.core, window, cmd) {
            tracing::error!(%window, error = %e, "a window command failed");
        }
    }

    fn set_page_scripts(&mut self, _scripts: PageScriptSet) {}

    fn clear_data(&mut self, _req: ClearDataRequest, done: Callback<Result<(), EngineError>>) {
        done(Err(EngineError::Unsupported(
            "site data before the WebKit adapter (M2-T02)",
        )));
    }

    fn set_proxy(&mut self, _proxy: ProxyConfig) {}
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
                core.send(Input::Engine(EngineEvent::WindowCreated { window: id }));
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
    let headless = headless();
    if headless {
        builder = builder.visible(false).focused(false);
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
    if !headless {
        window.set_focus()?;
    }
    Ok(())
}

/// End-to-end runs on a developer's Mac create real windows but keep them
/// hidden and unfocused (`test-hooks` builds with `MADO_TEST_HEADLESS`).
fn headless() -> bool {
    cfg!(feature = "test-hooks") && std::env::var_os("MADO_TEST_HEADLESS").is_some()
}
