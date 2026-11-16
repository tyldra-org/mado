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

mod blocker;
mod certificate;
mod convert;
mod delegate;
mod download;
pub mod extensions;
mod fullscreen;
mod keys;

pub use keys::{set_reserved as set_reserved_shortcuts, synthesize as synthesize_key};
mod menu;
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

pub use blocker::set_blocker_rules;
pub use convert::{has_geolocation_delegate, safari_version, webkit_bundle_version};
pub use menu::Labels;
pub use scheme::{Resolved, SchemeResolver};
pub use view::{TabView, TabViewInfo, synthesize_click};

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
    /// `en.json` labels for the context menu.
    pub labels: menu::Labels,
    /// Where native messaging hosts are looked up, first match wins
    /// (spec 09 §9.1).
    pub native_host_dirs: Vec<std::path::PathBuf>,
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

/// Whether the tab's window shows a sheet (the certificate viewer), and
/// closes it, for end-to-end tests.
pub fn take_sheet(tab: TabId) -> bool {
    let Some(view) = with(|a| a.view(tab)).flatten() else {
        return false;
    };
    let Some(window) = view.window() else {
        return false;
    };
    match window.attachedSheet() {
        Some(sheet) => {
            window.endSheet(&sheet);
            true
        }
        None => false,
    }
}

/// Applies the profile's safety and privacy settings (spec 08 §1, §8).
pub fn set_privacy(privacy: mado_engine::PrivacyConfig) {
    with(|a| a.set_privacy(privacy));
}

/// Accepts `host`'s certificate for the session (spec 08 §3).
pub fn allow_certificate(host: String) {
    delegate::TRUSTED.with_borrow_mut(|t| t.insert(host));
}

/// Pauses, cancels or resumes a download (spec 12 §4.2).
pub fn download(
    id: mado_engine::event::EngineDownloadId,
    cmd: mado_engine::event::DownloadCommand,
) {
    download::command(id, cmd, view::resume_view());
}

/// Before the app exits: answers questions still open (dialogs, prompts,
/// sign-in) with their safe defaults, as WebKit requires. Main thread.
pub fn shutdown() {
    delegate::answer_all_pending();
}

/// Removes the persistent website data store of `profile` (spec 12 §1.4).
/// No tab view of this process may use it. Main thread; `done` runs once.
pub fn remove_data_store(profile: ProfileId, done: Callback<Result<(), EngineError>>) {
    use objc2_foundation::{NSError, NSUUID};
    use objc2_web_kit::WKWebsiteDataStore;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        done(Err(EngineError::Failed("not on the main thread".into())));
        return;
    };
    let uuid = NSUUID::from_bytes(*profile.as_uuid().as_bytes());
    let done = std::cell::Cell::new(Some(done));
    let block = block2::RcBlock::new(move |error: *mut NSError| {
        // SAFETY: nil or a live error for the call.
        let error = unsafe { error.as_ref() };
        if let Some(done) = done.take() {
            done(match error {
                // WebKit reports a store that never existed as an error.
                Some(e) if e.code() != 1 => {
                    Err(EngineError::Failed(e.localizedDescription().to_string()))
                }
                _ => Ok(()),
            });
        }
    });
    // SAFETY: a completion block WebKit calls once on the main thread.
    unsafe {
        WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(&uuid, &block, mtm)
    };
}

/// Whether a persistent website data store exists for `profile`. Main
/// thread; `done` runs once.
pub fn has_data_store(profile: ProfileId, done: Callback<bool>) {
    use objc2_foundation::{NSArray, NSUUID};
    use objc2_web_kit::WKWebsiteDataStore;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        done(false);
        return;
    };
    let wanted = *profile.as_uuid().as_bytes();
    let done = std::cell::Cell::new(Some(done));
    let block = block2::RcBlock::new(move |ids: std::ptr::NonNull<NSArray<NSUUID>>| {
        // SAFETY: WebKit hands a live array.
        let ids = unsafe { ids.as_ref() };
        let found = ids.iter().any(|u| u.as_bytes() == wanted);
        if let Some(done) = done.take() {
            done(found);
        }
    });
    // SAFETY: as above.
    unsafe { WKWebsiteDataStore::fetchAllDataStoreIdentifiers(&block, mtm) };
}
