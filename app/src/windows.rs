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

    fn create_tab(&mut self, spec: TabSpec) -> Result<(), EngineError> {
        mado_engine_webkit::create_tab(spec)
    }

    fn destroy_tab(&mut self, tab: TabId) {
        mado_engine_webkit::destroy_tab(tab);
    }

    fn apply(&mut self, tab: TabId, cmd: TabCommand) -> Result<(), EngineError> {
        mado_engine_webkit::apply(tab, cmd)
    }

    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand) {
        if let Err(e) = apply(&self.app, &self.core, window, cmd) {
            tracing::error!(%window, error = %e, "a window command failed");
        }
    }

    fn set_page_scripts(&mut self, scripts: PageScriptSet) {
        mado_engine_webkit::set_page_scripts(scripts);
    }

    fn clear_data(&mut self, req: ClearDataRequest, done: Callback<Result<(), EngineError>>) {
        mado_engine_webkit::clear_data(req, done);
    }

    fn set_proxy(&mut self, proxy: ProxyConfig) {
        mado_engine_webkit::set_proxy(proxy);
    }

    fn allow_certificate(&mut self, host: String) {
        mado_engine_webkit::allow_certificate(host);
    }

    fn set_privacy(&mut self, privacy: mado_engine::PrivacyConfig) {
        mado_engine_webkit::set_privacy(privacy);
    }

    fn load_extension(
        &mut self,
        ext: mado_engine::ExtensionLoad,
        done: Callback<Result<(), EngineError>>,
    ) {
        mado_engine_webkit::extensions::load(ext, done);
    }

    fn unload_extension(&mut self, id: String) {
        mado_engine_webkit::extensions::unload(&id);
    }

    fn set_extension_windows(&mut self, windows: mado_engine::ExtensionWindows) {
        mado_engine_webkit::extensions::set_windows(windows);
    }

    fn download(
        &mut self,
        id: mado_engine::event::EngineDownloadId,
        cmd: mado_engine::event::DownloadCommand,
    ) {
        mado_engine_webkit::download(id, cmd);
    }
}

/// Where the adapter finds a window's content view.
struct Host {
    app: AppHandle,
}

impl mado_engine_webkit::WindowHost for Host {
    fn ns_window(&self, window: WindowId) -> Option<*mut std::ffi::c_void> {
        self.app.get_window(&window_label(window))?.ns_window().ok()
    }
}

/// Installs the WebKit adapter for this profile. Call on the main thread
/// before the core starts creating tabs.
pub fn install_webkit(
    app: &AppHandle,
    core: Arc<OnceLock<CoreHandle>>,
    profile: mado_types::ProfileId,
    ephemeral: bool,
    inspectable: bool,
) -> Result<(), String> {
    let mtm = objc2::MainThreadMarker::new().ok_or("WebKit starts on the main thread")?;
    let runner_app = app.clone();
    let runner: mado_engine_webkit::MainRunner = Arc::new(move |job| {
        if let Err(e) = runner_app.run_on_main_thread(job) {
            tracing::error!(error = %e, "the main thread is gone");
        }
    });
    let events = EventSink::new(move |event| {
        if let Some(core) = core.get() {
            core.send(Input::Engine(event));
        }
    });
    let assets = app.clone();
    let resolver: mado_engine_webkit::SchemeResolver =
        Arc::new(move |url: &str| resolve(&assets, url));
    mado_engine_webkit::install(
        mtm,
        mado_engine_webkit::Config {
            profile,
            host: Box::new(Host { app: app.clone() }),
            ephemeral,
            events,
            runner,
            resolver,
            inspectable,
            labels: {
                let strings = mado_settings::Strings::english();
                std::sync::Arc::new(move |key: &str| strings.get(key).to_owned())
            },
        },
    );
    Ok(())
}

/// `mado://<page>/...` (spec 01 §4.4): every internal page is the
/// internal-page entry of the UI bundle, which reads its page from the URL;
/// other paths are the bundle's assets (scripts, styles).
fn resolve(app: &AppHandle, url: &str) -> Option<mado_engine_webkit::Resolved> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "mado" {
        return None;
    }
    let path = parsed.path();
    let asset = if path.is_empty() || path == "/" {
        "internal.html"
    } else {
        path.trim_start_matches('/')
    };
    if asset.split('/').any(|part| part == "..") {
        return None;
    }
    let found = app.asset_resolver().get(asset.to_owned())?;
    Some(mado_engine_webkit::Resolved {
        mime: found.mime_type,
        body: found.bytes,
    })
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
    if bounds.width > 0.0 && on_a_screen(app, bounds) {
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

/// Whether a saved window's top-left corner is on a screen that still
/// exists. A window whose screen is gone opens on the main screen with its
/// size (spec 04 §8 rule 5).
fn on_a_screen(app: &AppHandle, b: &Bounds) -> bool {
    app.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|m| {
            let scale = m.scale_factor();
            let pos = m.position().to_logical::<f64>(scale);
            let size = m.size().to_logical::<f64>(scale);
            b.x >= pos.x && b.y >= pos.y && b.x < pos.x + size.width && b.y < pos.y + size.height
        })
    })
}

/// End-to-end runs on a developer's Mac create real windows but keep them
/// hidden and unfocused (`test-hooks` builds with `MADO_TEST_HEADLESS`).
fn headless() -> bool {
    cfg!(feature = "test-hooks") && std::env::var_os("MADO_TEST_HEADLESS").is_some()
}
