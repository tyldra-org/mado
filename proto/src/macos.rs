//! macOS tab views: `WKWebView`s added to the Tauri window's content view.
//!
//! WebKit objects live on the main thread only. Code on other threads names a
//! tab by its `TabId` and reaches it through `shell::on_main`; navigation
//! results come back through a condition variable, so experiments read as
//! sequential steps.

use std::cell::RefCell;
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_foundation::{
    NSBundle, NSError, NSPoint, NSProcessInfo, NSRect, NSSize, NSString, NSURL, NSURLRequest,
    ns_string,
};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView, WKWebViewConfiguration};
use serde::Serialize;
use serde_json::{Value, json};

use crate::shell::Rect;

pub type TabId = usize;

struct Tab {
    view: Retained<WKWebView>,
    // The web view holds its navigation delegate weakly; this keeps it alive.
    _delegate: Retained<NavigationDelegate>,
}

thread_local! {
    // Only the main thread ever touches this list.
    static TABS: RefCell<Vec<Tab>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Navigation {
    Finished { url: String },
    Failed { url: String, error: String },
}

static NAVIGATIONS: Mutex<Vec<(TabId, Navigation)>> = Mutex::new(Vec::new());
static NAVIGATED: Condvar = Condvar::new();

fn record_navigation(tab: TabId, navigation: Navigation) {
    if let Ok(mut list) = NAVIGATIONS.lock() {
        list.push((tab, navigation));
        NAVIGATED.notify_all();
    }
}

/// Forgets navigation results already reported for `tab`.
pub fn clear_navigation(tab: TabId) {
    if let Ok(mut list) = NAVIGATIONS.lock() {
        list.retain(|(t, _)| *t != tab);
    }
}

/// Waits for the next finished or failed navigation of `tab`.
pub fn wait_navigation(tab: TabId, timeout: Duration) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    let mut list = NAVIGATIONS.lock().map_err(|_| "navigation lock poisoned")?;
    loop {
        if let Some(i) = list.iter().position(|(t, _)| *t == tab) {
            let (_, navigation) = list.remove(i);
            return match navigation {
                Navigation::Finished { .. } => {
                    serde_json::to_value(&navigation).map_err(|e| e.to_string())
                }
                Navigation::Failed { url, error } => {
                    Err(format!("navigation to {url} failed: {error}"))
                }
            };
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!(
                "tab {tab}: no navigation finished within {timeout:?}"
            ));
        }
        list = NAVIGATED
            .wait_timeout(list, left)
            .map_err(|_| "navigation lock poisoned")?
            .0;
    }
}

fn url_of(view: &WKWebView) -> String {
    // SAFETY: reading the URL property of a live web view on the main thread.
    unsafe { view.URL() }
        .and_then(|url| url.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn describe_error(error: &NSError) -> String {
    format!(
        "{} {} ({})",
        error.domain(),
        error.code(),
        error.localizedDescription()
    )
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoProtoNavigationDelegate"]
    #[ivars = TabId]
    struct NavigationDelegate;

    unsafe impl NSObjectProtocol for NavigationDelegate {}

    unsafe impl WKNavigationDelegate for NavigationDelegate {
        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            record_navigation(*self.ivars(), Navigation::Finished { url: url_of(view) });
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            let failed = Navigation::Failed {
                url: url_of(view),
                error: describe_error(error),
            };
            record_navigation(*self.ivars(), failed);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            let failed = Navigation::Failed {
                url: url_of(view),
                error: describe_error(error),
            };
            record_navigation(*self.ivars(), failed);
        }
    }
);

impl NavigationDelegate {
    fn new(mtm: MainThreadMarker, tab: TabId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(tab);
        // SAFETY: NSObject's designated initializer takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

fn main_thread() -> Result<MainThreadMarker, String> {
    MainThreadMarker::new().ok_or_else(|| "WebKit is touched only on the main thread".into())
}

fn content_view(window: &tauri::Window) -> Result<Retained<NSView>, String> {
    let ns_window = window
        .ns_window()
        .map_err(|e| e.to_string())?
        .cast::<NSWindow>();
    // SAFETY: Tauri returns the live NSWindow of this window; retaining it
    // keeps it valid while we use it.
    let ns_window = unsafe { Retained::retain(ns_window) }.ok_or("Tauri returned no NSWindow")?;
    ns_window
        .contentView()
        .ok_or_else(|| "the window has no content view".into())
}

/// Converts a CSS-pixel rectangle from the UI (top-left origin) to a frame in
/// the content view, which may count `y` from the bottom.
/// The UI webview: the bottom subview of the content view, since Tauri adds
/// it before any tab view exists.
fn ui_view(content: &NSView) -> Result<Retained<NSView>, String> {
    content
        .subviews()
        .firstObject()
        .ok_or_else(|| "no UI webview".into())
}

/// Converts a rectangle in the UI webview's CSS pixels to a frame in the
/// content view.
///
/// CSS (0, 0) is not the view's top-left corner: Tauri windows use a
/// full-size content view, and WebKit starts the UI's CSS viewport below the
/// title bar's safe area (32 points on macOS 27). Converting through the UI
/// view also handles the content view counting `y` from the bottom.
fn frame_in(content: &NSView, rect: Rect) -> Result<NSRect, String> {
    let ui = ui_view(content)?;
    let top = ui.safeAreaInsets().top;
    let y = if ui.isFlipped() {
        rect.y + top
    } else {
        ui.bounds().size.height - top - rect.y - rect.height
    };
    let in_ui = NSRect::new(
        NSPoint::new(rect.x, y),
        NSSize::new(rect.width, rect.height),
    );
    Ok(ui.convertRect_toView(in_ui, Some(content)))
}

fn load(view: &WKWebView, url: &str) -> Result<(), String> {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| format!("not a URL: {url}"))?;
    let request = NSURLRequest::requestWithURL(&ns_url);
    // SAFETY: loading a request in a live web view on the main thread.
    unsafe { view.loadRequest(&request) };
    Ok(())
}

/// Creates a tab view over `rect`, above the UI webview, and loads `url`.
pub fn create_tab(window: &tauri::Window, rect: Rect, url: &str) -> Result<TabId, String> {
    let mtm = main_thread()?;
    let content = content_view(window)?;
    // SAFETY: creating a default configuration on the main thread.
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    let frame = frame_in(&content, rect)?;
    // SAFETY: a fresh configuration and a finite frame, on the main thread.
    let view = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
    };
    let id = TABS.with_borrow(Vec::len);
    let delegate = NavigationDelegate::new(mtm, id);
    // SAFETY: the delegate outlives the view's use of it: both live in TABS.
    unsafe {
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        view.setInspectable(true);
    }
    // Both margins stay fixed while width and height follow the window, so a
    // live resize keeps the page aligned before the UI reports the new rect.
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content.addSubview(&view);
    let loaded = load(&view, url);
    TABS.with_borrow_mut(|tabs| {
        tabs.push(Tab {
            view,
            _delegate: delegate,
        })
    });
    loaded.map(|()| id)
}

fn with_tab<R>(tab: TabId, f: impl FnOnce(&WKWebView) -> R) -> Result<R, String> {
    main_thread()?;
    TABS.with_borrow(|tabs| tabs.get(tab).map(|t| f(&t.view)))
        .ok_or_else(|| format!("no tab {tab}"))
}

pub fn navigate(tab: TabId, url: &str) -> Result<(), String> {
    with_tab(tab, |view| load(view, url))?
}

pub fn set_frame(window: &tauri::Window, tab: TabId, rect: Rect) -> Result<(), String> {
    let content = content_view(window)?;
    let frame = frame_in(&content, rect)?;
    with_tab(tab, |view| view.setFrame(frame))
}

/// The content view's subviews, bottom to top, with class and frame.
pub fn describe_views(window: &tauri::Window) -> Result<Value, String> {
    let content = content_view(window)?;
    let views: Vec<Value> = content
        .subviews()
        .iter()
        .map(|view| {
            let f = view.frame();
            json!({
                "class": view.class().name().to_string_lossy(),
                "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
                "hidden": view.isHidden(),
            })
        })
        .collect();
    let b = content.bounds();
    let insets = |v: &NSView| {
        let i = v.safeAreaInsets();
        [i.top, i.left, i.bottom, i.right]
    };
    let ui = ui_view(&content)?;
    let window_mask = content.window().map(|w| w.styleMask().0);
    Ok(json!({
        "contentView": {
            "flipped": content.isFlipped(),
            "size": [b.size.width, b.size.height],
            "safeArea": insets(&content),
            "windowStyleMask": window_mask,
        },
        "uiSafeArea": insets(&ui),
        "bottomToTop": views,
    }))
}

/// Captures `window` with `screencapture`, which needs the Screen Recording
/// permission of the process that started the prototype.
pub fn screenshot(app: &tauri::AppHandle, label: &str, path: &Path) -> Result<String, String> {
    use tauri::Manager;
    let handle = app.clone();
    let label = label.to_owned();
    let number = crate::shell::on_main(app, move || {
        let window = handle.get_window(&label).ok_or("no window")?;
        let ns_window = window
            .ns_window()
            .map_err(|e| e.to_string())?
            .cast::<NSWindow>();
        // SAFETY: Tauri returns the live NSWindow of this window.
        let ns_window = unsafe { Retained::retain(ns_window) }.ok_or("no NSWindow")?;
        Ok::<_, String>(ns_window.windowNumber())
    })??;
    let status = std::process::Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{number}"))
        .arg(path)
        .status()
        .map_err(|e| format!("screencapture: {e}"))?;
    if !status.success() || !path.exists() {
        return Err(format!(
            "screencapture failed ({status}); grant Screen Recording?"
        ));
    }
    Ok(path.display().to_string())
}

/// OS and WebKit versions, recorded with every result.
pub fn environment() -> Value {
    let os = NSProcessInfo::processInfo()
        .operatingSystemVersionString()
        .to_string();
    // SAFETY: WKWebView is a real class loaded from WebKit.framework.
    let bundle = unsafe { NSBundle::bundleForClass(WKWebView::class()) };
    let version = |key: &NSString| {
        bundle
            .objectForInfoDictionaryKey(key)
            .and_then(|v| v.downcast::<NSString>().ok())
            .map(|s| s.to_string())
    };
    json!({
        "os": format!("macOS {os}"),
        "webkit": version(ns_string!("CFBundleVersion")),
        "webkitShort": version(ns_string!("CFBundleShortVersionString")),
        "arch": std::env::consts::ARCH,
    })
}
