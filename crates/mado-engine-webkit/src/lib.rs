//! macOS engine adapter on WKWebView (spec 02 §4).
//!
//! WebKit objects live on the main thread only. The adapter keeps its tab
//! views in a main-thread state ([`with`]); the app's engine calls it from
//! `EngineOp::run`, which always runs on the main thread. Delegates report
//! to the core through the [`EventSink`]; decisions the core answers later
//! come back through the app's main-thread runner, and every one has a
//! 30-second safe default.
//!
//! Roadmap: M2-T02.

#![cfg(target_os = "macos")]

mod convert;
mod delegate;
mod scheme;
mod timeouts;
mod view;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use mado_engine::{
    Callback, ClearDataRequest, EngineError, EventSink, PageScriptSet, ProxyConfig, TabCommand,
    TabSpec,
};
use mado_types::{ProfileId, TabId, WindowId};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;

pub use convert::safari_version;
pub use scheme::{Resolved, SchemeResolver};
pub use view::{TabView, TabViewInfo, download};

/// Runs a closure on the main thread (the app: Tauri's `run_on_main_thread`).
pub type MainRunner = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// Where tab views go: the app's windows.
pub trait WindowHost {
    /// The live `NSWindow` of `window` (Tauri's `ns_window()`). Its content
    /// view's first subview is the UI webview; tab views go directly above
    /// it, below the overlay webview.
    fn ns_window(&self, window: WindowId) -> Option<*mut std::ffi::c_void>;
}

/// The content view of a window the host names.
pub(crate) fn content_view(host: &dyn WindowHost, window: WindowId) -> Option<Retained<NSView>> {
    let ptr = host.ns_window(window)?.cast::<objc2_app_kit::NSWindow>();
    // SAFETY: the host returns a live NSWindow or none; retaining it keeps it
    // valid while we use it, on the main thread.
    let ns_window = unsafe { Retained::retain(ptr) }?;
    ns_window.contentView()
}

/// What the adapter needs once per profile process.
pub struct Config {
    pub profile: ProfileId,
    pub host: Box<dyn WindowHost>,
    /// Private and guest profiles keep nothing on disk.
    pub ephemeral: bool,
    pub events: EventSink,
    pub runner: MainRunner,
    pub resolver: SchemeResolver,
    /// `developer.inspectPages`.
    pub inspectable: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("profile", &self.profile)
            .field("ephemeral", &self.ephemeral)
            .finish_non_exhaustive()
    }
}

thread_local! {
    static ADAPTER: RefCell<Option<view::Adapter>> = const { RefCell::new(None) };
}

/// Creates the adapter on the main thread. Call once, before any tab.
pub fn install(mtm: MainThreadMarker, config: Config) {
    ADAPTER.with_borrow_mut(|a| *a = Some(view::Adapter::new(mtm, config)));
}

/// Runs `f` with the adapter. `None` before [`install`] or off the main
/// thread.
pub fn with<R>(f: impl FnOnce(&mut view::Adapter) -> R) -> Option<R> {
    MainThreadMarker::new()?;
    ADAPTER.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// The tab views a window holds, for the extension host (M3-T04).
pub fn tabs_in(window: WindowId) -> Vec<TabId> {
    with(|a| a.tabs_in(window)).unwrap_or_default()
}

/// Every tab view with its window, for the extension host (M3-T04).
pub fn tab_cache() -> BTreeMap<TabId, TabViewInfo> {
    with(|a| a.cache()).unwrap_or_default()
}

/// The engine calls, for the app's `Engine` implementation.
pub fn create_tab(spec: TabSpec) -> Result<(), EngineError> {
    with(|a| a.create_tab(spec)).unwrap_or(Err(EngineError::Unsupported("WebKit is not installed")))
}

pub fn destroy_tab(tab: TabId) {
    with(|a| a.destroy_tab(tab));
}

pub fn apply(tab: TabId, cmd: TabCommand) -> Result<(), EngineError> {
    view::apply(tab, cmd)
}

pub fn set_page_scripts(scripts: PageScriptSet) {
    with(|a| a.set_page_scripts(scripts));
}

pub fn clear_data(req: ClearDataRequest, done: Callback<Result<(), EngineError>>) {
    match MainThreadMarker::new() {
        Some(_) => view::clear_data(req, done),
        None => done(Err(EngineError::Unsupported("WebKit off the main thread"))),
    }
}

pub fn set_proxy(proxy: ProxyConfig) {
    with(|a| a.set_proxy(&proxy));
}
