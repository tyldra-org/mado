//! macOS tab views: `WKWebView`s added to the Tauri window's content view.
//!
//! WebKit objects live on the main thread only. Code on other threads names a
//! tab by its `TabId` and reaches it through `shell::on_main`; navigation
//! results come back through a condition variable, so experiments read as
//! sequential steps.

use std::cell::RefCell;
use std::path::Path;
use std::sync::{Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow,
    NSWindowOrderingMode,
};
use objc2_foundation::{
    NSBundle, NSData, NSError, NSPoint, NSProcessInfo, NSRect, NSSize, NSString, NSURL,
    NSURLRequest, ns_string,
};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKNavigation, WKNavigationDelegate,
    WKUserScript, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
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
    // Only the main thread ever touches these.
    // Index = TabId; a closed tab leaves `None`.
    static TABS: RefCell<Vec<Option<Tab>>> = const { RefCell::new(Vec::new()) };
    static OVERLAY: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Navigation {
    Finished { url: String },
    Failed { url: String, error: String },
}

static NAVIGATIONS: Mutex<Vec<(TabId, Navigation)>> = Mutex::new(Vec::new());

/// One call of the private content-rule-action delegate method.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleAction {
    pub tab: TabId,
    pub list: String,
    pub url: String,
    pub blocked_load: Option<bool>,
}

pub static RULE_ACTIONS: Mutex<Vec<RuleAction>> = Mutex::new(Vec::new());
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

        // WebKit's private SPI for applied content rule actions (10 §4.7).
        // WebKit calls it only if the delegate responds to it.
        #[unsafe(method(_webView:contentRuleListWithIdentifier:performedAction:forURL:))]
        fn rule_action(
            &self,
            _view: &WKWebView,
            identifier: &NSString,
            action: &AnyObject,
            url: &NSURL,
        ) {
            let responds = |name: objc2::runtime::Sel| -> bool {
                // SAFETY: respondsToSelector: is safe on any object.
                unsafe { msg_send![action, respondsToSelector: name] }
            };
            let blocked_load = responds(objc2::sel!(blockedLoad)).then(|| {
                // SAFETY: `_WKContentRuleListAction.blockedLoad` is a BOOL.
                let b: bool = unsafe { msg_send![action, blockedLoad] };
                b
            });
            if let Ok(mut list) = RULE_ACTIONS.lock() {
                list.push(RuleAction {
                    tab: *self.ivars(),
                    list: identifier.to_string(),
                    url: url
                        .absoluteString()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    blocked_load,
                });
            }
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
    let content = content_view(window)?;
    let frame = frame_in(&content, rect)?;
    let id = insert_tab(&content, frame)?;
    with_tab(id, |view| load(view, url))??;
    Ok(id)
}

/// Adds a tab view with the first tab's frame, hidden if `hidden`, as a
/// background tab is (01 §5.1). It loads nothing.
pub fn add_tab(window: &tauri::Window, hidden: bool) -> Result<TabId, String> {
    let content = content_view(window)?;
    let frame = with_tab(0, |v| v.frame())?;
    let id = insert_tab(&content, frame)?;
    with_tab(id, |v| v.setHidden(hidden))?;
    Ok(id)
}

extern "C-unwind" fn keep_frame(
    _this: *mut AnyObject,
    _cmd: objc2::runtime::Sel,
    frame: NSRect,
    _screen: *mut AnyObject,
) -> NSRect {
    frame
}

/// Moves the window to (x, y), entirely off every screen if asked.
///
/// AppKit keeps part of a titled window on some screen: it calls
/// `constrainFrameRect:toScreen:` when it orders the window in and on every
/// frame change, and pulled (-20000, -20000) back to x = -2870, 40 points
/// onto a second display. The prototype replaces `constrainFrameRect:toScreen:`
/// on the window's declared class (Tauri's `TaoWindow`; swapping the instance
/// to a subclass crashed, as KVO already subclasses it) so it returns the
/// frame unchanged. Mado's agent windows would be an NSWindow subclass with
/// that override.
pub fn move_window(window: &tauri::Window, x: f64, y: f64) -> Result<(f64, f64), String> {
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    // `-class` answers the declared class even when KVO has subclassed it.
    // SAFETY: -class takes no arguments and returns a class.
    let declared: *const objc2::runtime::AnyClass = unsafe { msg_send![&*w, class] };
    let keep: extern "C-unwind" fn(
        *mut AnyObject,
        objc2::runtime::Sel,
        NSRect,
        *mut AnyObject,
    ) -> NSRect = keep_frame;
    let types = c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@";
    // SAFETY: the implementation's signature matches the selector and the
    // type encoding; the class is a live NSWindow subclass.
    unsafe {
        objc2::ffi::class_replaceMethod(
            declared.cast_mut(),
            objc2::sel!(constrainFrameRect:toScreen:),
            std::mem::transmute::<
                extern "C-unwind" fn(
                    *mut AnyObject,
                    objc2::runtime::Sel,
                    NSRect,
                    *mut AnyObject,
                ) -> NSRect,
                objc2::runtime::Imp,
            >(keep),
            types.as_ptr(),
        );
    }
    w.setFrameOrigin(NSPoint::new(x, y));
    let f = w.frame();
    Ok((f.origin.x, f.origin.y))
}

/// Makes an experiment window invisible and inert while it keeps rendering:
/// fully transparent, ignoring the mouse, above other windows so nothing
/// occludes it, on the main screen. Experiment 12 compares this placement
/// with an off-screen one.
pub fn hide_window(window: &tauri::Window) -> Result<(), String> {
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    w.setAlphaValue(0.0);
    w.setIgnoresMouseEvents(true);
    // NSFloatingWindowLevel.
    w.setLevel(3);
    w.setFrameOrigin(NSPoint::new(0.0, 0.0));
    Ok(())
}

/// Restores what `hide_window` changed, at (x, y).
pub fn opaque_window_at(window: &tauri::Window, x: f64, y: f64) -> Result<(), String> {
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    w.setAlphaValue(1.0);
    w.setLevel(0);
    move_window(window, x, y).map(|_| ())
}

/// Whether any part of the window lies on any screen.
fn on_any_screen(frame: NSRect, mtm: MainThreadMarker) -> bool {
    objc2_app_kit::NSScreen::screens(mtm).iter().any(|s| {
        let r = s.frame();
        frame.origin.x < r.origin.x + r.size.width
            && r.origin.x < frame.origin.x + frame.size.width
            && frame.origin.y < r.origin.y + r.size.height
            && r.origin.y < frame.origin.y + frame.size.height
    })
}

/// Where the window is and what AppKit thinks of it.
pub fn window_state(window: &tauri::Window) -> Result<Value, String> {
    let mtm = main_thread()?;
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    let f = w.frame();
    Ok(json!({
        "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
        "onAnyScreen": on_any_screen(f, mtm),
        "isKey": w.isKeyWindow(),
        "isVisible": w.isVisible(),
        "alpha": w.alphaValue(),
        "level": w.level(),
        "occlusionVisible": w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible),
    }))
}

/// Makes the tab view its window's first responder, so key events go to it.
pub fn focus_tab(window: &tauri::Window, tab: TabId) -> Result<(), String> {
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    let accepted = with_tab(tab, |view| w.makeFirstResponder(Some(view)))?;
    accepted
        .then_some(())
        .ok_or_else(|| "the tab view refused first responder".into())
}

/// ANSI virtual key codes for the lowercase letters `type_keys` supports.
fn key_code(c: char) -> Option<u16> {
    Some(match c {
        'a' => 0x00,
        's' => 0x01,
        'd' => 0x02,
        'f' => 0x03,
        'h' => 0x04,
        'g' => 0x05,
        'z' => 0x06,
        'x' => 0x07,
        'c' => 0x08,
        'v' => 0x09,
        'b' => 0x0B,
        'q' => 0x0C,
        'w' => 0x0D,
        'e' => 0x0E,
        'r' => 0x0F,
        'y' => 0x10,
        't' => 0x11,
        'o' => 0x1F,
        'u' => 0x20,
        'i' => 0x22,
        'p' => 0x23,
        'l' => 0x25,
        'j' => 0x26,
        'k' => 0x28,
        'n' => 0x2D,
        'm' => 0x2E,
        _ => return None,
    })
}

/// Types lowercase letters into a tab view with synthesised key-down and
/// key-up events sent to the view, never through the real keyboard.
pub fn type_keys(window: &tauri::Window, tab: TabId, text: &str) -> Result<(), String> {
    let content = content_view(window)?;
    let w = content.window().ok_or("no window")?;
    let number = w.windowNumber();
    for c in text.chars() {
        let code = key_code(c).ok_or_else(|| format!("no key code for {c:?}"))?;
        let chars = NSString::from_str(&c.to_string());
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let time = NSProcessInfo::processInfo().systemUptime();
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                time,
                number,
                None,
                &chars,
                &chars,
                false,
                code,
            )
            .ok_or("NSEvent refused the key event")?;
            with_tab(tab, |view| {
                if kind == NSEventType::KeyDown {
                    view.keyDown(&event);
                } else {
                    view.keyUp(&event);
                }
            })?;
        }
    }
    Ok(())
}

thread_local! {
    static RULE_STORE: RefCell<Option<Retained<WKContentRuleListStore>>> = const { RefCell::new(None) };
    static RULE_LISTS: RefCell<Vec<(String, Retained<WKContentRuleList>)>> = const { RefCell::new(Vec::new()) };
}

/// Opens a content rule list store in `dir`, so compiles there are cold.
pub fn open_rule_store(dir: &Path) -> Result<(), String> {
    let mtm = main_thread()?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
    // SAFETY: a file URL to a directory we own, on the main thread.
    let store = unsafe { WKContentRuleListStore::storeWithURL(Some(&url), mtm) }
        .ok_or("WebKit refused the store directory")?;
    RULE_STORE.with_borrow_mut(|s| *s = Some(store));
    Ok(())
}

type Done = mpsc::Receiver<Result<(), String>>;

/// Compiles `json` as list `id` in the open store. The receiver gets the
/// outcome; the compiled list is kept for `apply_rules`.
pub fn compile_rules(id: &str, json: &str) -> Result<Done, String> {
    main_thread()?;
    let store = RULE_STORE
        .with_borrow(Clone::clone)
        .ok_or("no rule store")?;
    let (tx, rx) = mpsc::channel();
    let name = id.to_owned();
    let block = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
        // SAFETY: WebKit passes a valid list or a valid error.
        let outcome = match (unsafe { Retained::retain(list) }, unsafe { error.as_ref() }) {
            (Some(list), _) => {
                RULE_LISTS.with_borrow_mut(|l| l.push((name.clone(), list)));
                Ok(())
            }
            (None, Some(error)) => Err(describe_error(error)),
            (None, None) => Err("no list and no error".into()),
        };
        let _ = tx.send(outcome);
    });
    // SAFETY: strings and a block that only stores the list and sends.
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str(id)),
            Some(&NSString::from_str(json)),
            Some(&block),
        );
    }
    Ok(rx)
}

/// Looks up list `id` in the open store without compiling.
pub fn lookup_rules(id: &str) -> Result<Done, String> {
    main_thread()?;
    let store = RULE_STORE
        .with_borrow(Clone::clone)
        .ok_or("no rule store")?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
        // SAFETY: as in compile_rules.
        let outcome = match (list.is_null(), unsafe { error.as_ref() }) {
            (false, _) => Ok(()),
            (true, Some(error)) => Err(describe_error(error)),
            (true, None) => Err("not found".into()),
        };
        let _ = tx.send(outcome);
    });
    // SAFETY: as in compile_rules.
    unsafe {
        store.lookUpContentRuleListForIdentifier_completionHandler(
            Some(&NSString::from_str(id)),
            Some(&block),
        );
    }
    Ok(rx)
}

/// Adds compiled list `id` to the tab's user content controller.
pub fn apply_rules(tab: TabId, id: &str) -> Result<(), String> {
    let list = RULE_LISTS
        .with_borrow(|l| {
            l.iter()
                .find(|(n, _)| n == id)
                .map(|(_, list)| list.clone())
        })
        .ok_or_else(|| format!("list {id} is not compiled"))?;
    with_tab(tab, |view| {
        // SAFETY: configuration and controller of a live web view.
        unsafe {
            view.configuration()
                .userContentController()
                .addContentRuleList(&list);
        }
    })
}

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    // <Network/endpoint.h> and <Network/proxy_config.h> (macOS 14+). Both
    // return retained objects.
    fn nw_endpoint_create_host(
        hostname: *const std::ffi::c_char,
        port: *const std::ffi::c_char,
    ) -> *mut AnyObject;
    fn nw_proxy_config_create_http_connect(
        endpoint: *mut AnyObject,
        tls_options: *mut AnyObject,
    ) -> *mut AnyObject;
}

/// Adds a hidden tab view with its own non-persistent data store, as a
/// profile has its own store, optionally sending all its traffic through an
/// HTTP CONNECT proxy at 127.0.0.1:`proxy_port`
/// (`WKWebsiteDataStore.proxyConfigurations`, 13 network settings).
pub fn add_tab_with_store(
    window: &tauri::Window,
    proxy_port: Option<u16>,
) -> Result<TabId, String> {
    let mtm = main_thread()?;
    let content = content_view(window)?;
    let frame = with_tab(0, |v| v.frame())?;
    // SAFETY: plain constructors on the main thread.
    let (configuration, store) = unsafe {
        (
            WKWebViewConfiguration::new(mtm),
            WKWebsiteDataStore::nonPersistentDataStore(mtm),
        )
    };
    if let Some(port) = proxy_port {
        let host = std::ffi::CString::new("127.0.0.1").map_err(|e| e.to_string())?;
        let port = std::ffi::CString::new(port.to_string()).map_err(|e| e.to_string())?;
        // SAFETY: valid C strings; both functions return +1 objects, which
        // Retained::from_raw takes over.
        let config = unsafe {
            let endpoint = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
            let endpoint = Retained::from_raw(endpoint).ok_or("nw_endpoint_create_host failed")?;
            let raw = nw_proxy_config_create_http_connect(
                Retained::as_ptr(&endpoint).cast_mut(),
                std::ptr::null_mut(),
            );
            Retained::from_raw(raw).ok_or("nw_proxy_config_create_http_connect failed")?
        };
        let list = objc2_foundation::NSArray::from_retained_slice(&[config]);
        let selector = objc2::sel!(setProxyConfigurations:);
        // SAFETY: respondsToSelector: is safe on any object.
        let responds: bool = unsafe { msg_send![&*store, respondsToSelector: selector] };
        if !responds {
            return Err("WKWebsiteDataStore has no proxyConfigurations".into());
        }
        // SAFETY: the property takes an NSArray of nw_proxy_config_t.
        let () = unsafe { msg_send![&*store, setProxyConfigurations: &*list] };
    }
    // SAFETY: setting the store on a fresh configuration.
    unsafe { configuration.setWebsiteDataStore(&store) };
    let id = insert_tab_with(&content, frame, &configuration)?;
    with_tab(id, |v| v.setHidden(true))?;
    Ok(id)
}

/// Adds a user script that runs at document start in the page world, as
/// Mado's page scripts are.
pub fn add_user_script(tab: TabId, source: &str) -> Result<(), String> {
    let mtm = main_thread()?;
    // SAFETY: a fresh user script from a string, on the main thread.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            WKUserScript::alloc(mtm),
            &NSString::from_str(source),
            objc2_web_kit::WKUserScriptInjectionTime::AtDocumentStart,
            true,
        )
    };
    with_tab(tab, |view| {
        // SAFETY: the controller of a live web view.
        unsafe {
            view.configuration()
                .userContentController()
                .addUserScript(&script)
        };
    })
}

pub fn reload(tab: TabId) -> Result<(), String> {
    with_tab(tab, |view| {
        // SAFETY: a navigation call on a live web view.
        unsafe { view.reload() };
    })
}

/// The tab's page as `.webarchive` bytes.
pub fn web_archive(tab: TabId) -> Result<mpsc::Receiver<Result<Vec<u8>, String>>, String> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
        // SAFETY: WebKit passes valid data or a valid error.
        let outcome = match (unsafe { data.as_ref() }, unsafe { error.as_ref() }) {
            (Some(data), _) => Ok(data.to_vec()),
            (None, Some(error)) => Err(describe_error(error)),
            (None, None) => Err("no data and no error".into()),
        };
        let _ = tx.send(outcome);
    });
    // SAFETY: a live web view and a block that only sends.
    with_tab(tab, |view| unsafe {
        view.createWebArchiveDataWithCompletionHandler(&block)
    })?;
    Ok(rx)
}

/// Loads `bytes` as a document of type `mime`.
pub fn load_data(tab: TabId, bytes: &[u8], mime: &str, base: &str) -> Result<(), String> {
    let data = NSData::with_bytes(bytes);
    let base = NSURL::URLWithString(&NSString::from_str(base)).ok_or("bad base URL")?;
    with_tab(tab, |view| {
        // SAFETY: loading data into a live web view on the main thread.
        unsafe {
            view.loadData_MIMEType_characterEncodingName_baseURL(
                &data,
                &NSString::from_str(mime),
                &NSString::from_str("utf-8"),
                &base,
            )
        };
    })
}

/// WebKit's public find: whether `text` is on the page. It reports no count.
pub fn find(tab: TabId, text: &str) -> Result<mpsc::Receiver<bool>, String> {
    let mtm = main_thread()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(
        move |result: std::ptr::NonNull<objc2_web_kit::WKFindResult>| {
            // SAFETY: WebKit passes a valid result.
            let _ = tx.send(unsafe { result.as_ref().matchFound() });
        },
    );
    // SAFETY: a fresh configuration; a block that only sends.
    let configuration = unsafe { objc2_web_kit::WKFindConfiguration::new(mtm) };
    with_tab(tab, |view| unsafe {
        view.findString_withConfiguration_completionHandler(
            &NSString::from_str(text),
            Some(&configuration),
            &block,
        );
    })?;
    Ok(rx)
}

/// Class names of every view inside the tab view, to spot WebKit's own
/// warning views.
pub fn view_classes(tab: TabId) -> Result<Vec<String>, String> {
    fn walk(view: &NSView, out: &mut Vec<String>) {
        out.push(view.class().name().to_string_lossy().into_owned());
        for child in view.subviews().iter() {
            walk(&child, out);
        }
    }
    with_tab(tab, |view| {
        let mut out = Vec::new();
        walk(view, &mut out);
        out
    })
}

/// Shows a hidden tab view, as activating a background tab does.
pub fn show_tab(tab: TabId) -> Result<(), String> {
    with_tab(tab, |view| view.setHidden(false))
}

/// Removes a tab view and releases it, as sleeping or closing a tab does.
pub fn close_tab(tab: TabId) -> Result<(), String> {
    main_thread()?;
    let closed = TABS.with_borrow_mut(|tabs| tabs.get_mut(tab).and_then(Option::take));
    let closed = closed.ok_or_else(|| format!("no tab {tab}"))?;
    closed.view.removeFromSuperview();
    Ok(())
}

fn insert_tab(content: &NSView, frame: NSRect) -> Result<TabId, String> {
    let mtm = main_thread()?;
    // SAFETY: creating a default configuration on the main thread.
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    insert_tab_with(content, frame, &configuration)
}

fn insert_tab_with(
    content: &NSView,
    frame: NSRect,
    configuration: &WKWebViewConfiguration,
) -> Result<TabId, String> {
    let mtm = main_thread()?;
    // SAFETY: a fresh configuration and a finite frame, on the main thread.
    let view = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, configuration)
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
    // Tab views sit below the overlay (01 §5.1).
    match OVERLAY.with_borrow(Clone::clone) {
        Some(overlay) => {
            content.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Below,
                Some(&overlay),
            );
        }
        None => content.addSubview(&view),
    }
    TABS.with_borrow_mut(|tabs| {
        tabs.push(Some(Tab {
            view,
            _delegate: delegate,
        }))
    });
    Ok(id)
}

fn with_tab<R>(tab: TabId, f: impl FnOnce(&WKWebView) -> R) -> Result<R, String> {
    main_thread()?;
    TABS.with_borrow(|tabs| tabs.get(tab).and_then(Option::as_ref).map(|t| f(&t.view)))
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
            "screencapture failed ({status}): experiment windows are transparent, or Screen Recording is not granted"
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

/// Records the overlay's native view. Tauri adds a child webview on top of
/// the content view at once, so right after `add_child` it is the topmost
/// subview. Experiment 1 checks this against what `Webview::with_webview`
/// returns.
pub fn register_overlay(window: &tauri::Window) -> Result<(), String> {
    main_thread()?;
    let content = content_view(window)?;
    let top = content.subviews().lastObject().ok_or("no subviews")?;
    OVERLAY.with_borrow_mut(|o| *o = Some(top));
    Ok(())
}

/// The overlay's native view address, to compare with Tauri's.
pub fn overlay_address() -> Result<usize, String> {
    main_thread()?;
    OVERLAY
        .with_borrow(|o| o.as_ref().map(|v| Retained::as_ptr(v) as usize))
        .ok_or_else(|| "no overlay".into())
}

/// Names a direct subview of the content view: `ui`, `overlay`, `tab:<n>`.
fn role(content: &NSView, view: &NSView) -> String {
    let same = |a: &NSView, b: &NSView| std::ptr::eq(a, b);
    if ui_view(content).is_ok_and(|ui| same(&ui, view)) {
        return "ui".into();
    }
    if OVERLAY.with_borrow(|o| o.as_deref().is_some_and(|o| same(o, view))) {
        return "overlay".into();
    }
    TABS.with_borrow(|tabs| {
        tabs.iter()
            .position(|t| t.as_ref().is_some_and(|t| same(&t.view, view)))
            .map_or_else(
                || format!("other:{}", view.class().name().to_string_lossy()),
                |i| format!("tab:{i}"),
            )
    })
}

/// The roles of the content view's subviews, bottom to top.
pub fn view_stack(window: &tauri::Window) -> Result<Vec<String>, String> {
    let content = content_view(window)?;
    Ok(content
        .subviews()
        .iter()
        .map(|v| role(&content, &v))
        .collect())
}

/// A point in the UI's CSS pixels, in content view coordinates.
fn css_point(content: &NSView, x: f64, y: f64) -> Result<NSPoint, String> {
    let frame = frame_in(
        content,
        Rect {
            x,
            y,
            width: 0.0,
            height: 0.0,
        },
    )?;
    Ok(frame.origin)
}

/// Which layer AppKit's hit testing gives a click at a CSS point.
pub fn hit_test(window: &tauri::Window, x: f64, y: f64) -> Result<String, String> {
    let content = content_view(window)?;
    let point = css_point(&content, x, y)?;
    // SAFETY: walking the view tree of a live window on the main thread.
    let superview = unsafe { content.superview() };
    let in_super = content.convertPoint_toView(point, superview.as_deref());
    let mut hit = content.hitTest(in_super).ok_or("nothing at that point")?;
    // Climb to the direct subview of the content view that holds the hit.
    // SAFETY: as above.
    while let Some(parent) = unsafe { hit.superview() } {
        if std::ptr::eq(&*parent, &*content) {
            return Ok(role(&content, &hit));
        }
        hit = parent;
    }
    Ok("content".into())
}

/// How a synthesised click reached the views.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    /// `NSWindow.sendEvent`: AppKit routes it as it routes a real click.
    /// WebKit drops these when the window is not key.
    Window,
    /// `mouseDown:` and `mouseUp:` sent to the view AppKit's hit test names,
    /// which works in a window that is not key (13 §5's route for agents).
    HitView,
}

/// Posts a left click at a CSS point. Never moves the real pointer.
pub fn click(window: &tauri::Window, x: f64, y: f64) -> Result<Delivery, String> {
    main_thread()?;
    let content = content_view(window)?;
    let in_content = css_point(&content, x, y)?;
    let point = content.convertPoint_toView(in_content, None);
    let ns_window = content.window().ok_or("no window")?;
    let delivery = if ns_window.isKeyWindow() {
        Delivery::Window
    } else {
        Delivery::HitView
    };
    // SAFETY: walking the view tree of a live window on the main thread.
    let superview = unsafe { content.superview() };
    let target = content
        .hitTest(content.convertPoint_toView(in_content, superview.as_deref()))
        .ok_or("nothing at that point")?;
    let number = ns_window.windowNumber();
    let time = NSProcessInfo::processInfo().systemUptime();
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            point,
            NSEventModifierFlags::empty(),
            time,
            number,
            None,
            0,
            1,
            1.0,
        )
        .ok_or("NSEvent refused the mouse event")?;
        match (delivery, kind) {
            (Delivery::Window, _) => ns_window.sendEvent(&event),
            (Delivery::HitView, NSEventType::LeftMouseDown) => target.mouseDown(&event),
            (Delivery::HitView, _) => target.mouseUp(&event),
        }
    }
    Ok(delivery)
}

fn frame_values(view: &NSView) -> [f64; 4] {
    let f = view.frame();
    [f.origin.x, f.origin.y, f.size.width, f.size.height]
}

/// The frames of the tab view and the overlay, in content view points.
pub fn frames(window: &tauri::Window, tab: TabId) -> Result<Value, String> {
    let _content = content_view(window)?;
    let tab_frame = with_tab(tab, |v| frame_values(v))?;
    let overlay = OVERLAY.with_borrow(Clone::clone).ok_or("no overlay")?;
    Ok(json!({
        "tab": tab_frame,
        "overlay": frame_values(&overlay),
        "overlayHidden": overlay.isHidden(),
    }))
}

/// Where a CSS rectangle of the UI lands in content view points.
pub fn expected_frame(window: &tauri::Window, rect: Rect) -> Result<[f64; 4], String> {
    let content = content_view(window)?;
    let f = frame_in(&content, rect)?;
    Ok([f.origin.x, f.origin.y, f.size.width, f.size.height])
}

/// The top safe-area inset WebKit gives the UI webview's CSS viewport.
pub fn ui_safe_top(window: &tauri::Window) -> Result<f64, String> {
    let content = content_view(window)?;
    Ok(ui_view(&content)?.safeAreaInsets().top)
}

/// Starts `body` (an async function body) in the page world of `view`. The
/// body's return value comes back as JSON on the receiver.
fn start_eval(view: &WKWebView, body: &str) -> mpsc::Receiver<Result<Value, String>> {
    let (tx, rx) = mpsc::channel();
    let wrapped = format!("return JSON.stringify(await (async () => {{ {body} }})() ?? null);");
    let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit passes either a valid error or a valid result.
        let outcome = if let Some(error) = unsafe { error.as_ref() } {
            Err(describe_error(error))
        } else {
            unsafe { result.as_ref() }
                .and_then(|r| r.downcast_ref::<NSString>())
                .map(|s| serde_json::from_str(&s.to_string()).map_err(|e| e.to_string()))
                .unwrap_or(Ok(Value::Null))
        };
        let _ = tx.send(outcome);
    });
    let mtm = view.mtm();
    // SAFETY: a live web view on the main thread, a function body string and
    // a completion block that only sends on a channel.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(&wrapped),
            None,
            None,
            &WKContentWorld::pageWorld(mtm),
            Some(&block),
        );
    }
    rx
}

/// Starts script in a tab's page world; see `start_eval`.
pub fn eval_tab(tab: TabId, body: &str) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
    with_tab(tab, |view| start_eval(view, body))
}

/// Starts script in the overlay's page world; see `start_eval`.
pub fn eval_overlay(body: &str) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
    main_thread()?;
    let overlay = OVERLAY.with_borrow(Clone::clone).ok_or("no overlay")?;
    let view = overlay
        .downcast_ref::<WKWebView>()
        .ok_or("the overlay is not a WKWebView")?;
    Ok(start_eval(view, body))
}

/// Loads `html` into a tab with `base` as its URL.
pub fn load_html(tab: TabId, html: &str, base: &str) -> Result<(), String> {
    let base = NSURL::URLWithString(&NSString::from_str(base)).ok_or("bad base URL")?;
    with_tab(tab, |view| {
        // SAFETY: loading a string into a live web view on the main thread.
        unsafe { view.loadHTMLString_baseURL(&NSString::from_str(html), Some(&base)) };
    })
}

/// Resizes the window's content area to `width` x `height` points at once,
/// the way a live resize step does.
pub fn resize(window: &tauri::Window, width: f64, height: f64) -> Result<(), String> {
    main_thread()?;
    let content = content_view(window)?;
    let ns_window = content.window().ok_or("no window")?;
    ns_window.setContentSize(NSSize::new(width, height));
    Ok(())
}

/// The tab's `interactionState` as bytes, and the class WebKit returned.
pub fn interaction_state(tab: TabId) -> Result<(Vec<u8>, String), String> {
    with_tab(tab, |view| {
        // SAFETY: reading a property of a live web view on the main thread.
        let state = unsafe { view.interactionState() }.ok_or("no interaction state")?;
        let class = state.class().name().to_string_lossy().into_owned();
        let data = state
            .downcast_ref::<NSData>()
            .ok_or_else(|| format!("interactionState is a {class}, not NSData"))?;
        Ok((data.to_vec(), class))
    })?
}

/// Restores bytes from `interaction_state` into a tab.
pub fn set_interaction_state(tab: TabId, bytes: &[u8]) -> Result<(), String> {
    let data = NSData::with_bytes(bytes);
    // SAFETY: WebKit copies the state; NSData is what it handed out.
    with_tab(tab, |view| unsafe { view.setInteractionState(Some(&data)) })
}

pub fn go_back(tab: TabId) -> Result<(), String> {
    // SAFETY: a navigation call on a live web view on the main thread.
    with_tab(tab, |view| {
        unsafe { view.goBack() };
    })
}

/// The tab's URL and its back and forward lists, oldest first.
pub fn history(tab: TabId) -> Result<Value, String> {
    with_tab(tab, |view| {
        // SAFETY: reading the back-forward list of a live web view.
        let list = unsafe { view.backForwardList() };
        let urls =
            |items: Retained<objc2_foundation::NSArray<objc2_web_kit::WKBackForwardListItem>>| {
                items
                    .iter()
                    .map(|item| {
                        unsafe { item.URL() }
                            .absoluteString()
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
            };
        json!({
            "url": url_of(view),
            "canGoBack": unsafe { view.canGoBack() },
            "canGoForward": unsafe { view.canGoForward() },
            "back": urls(unsafe { list.backList() }),
            "forward": urls(unsafe { list.forwardList() }),
        })
    })
}

/// The PID of the tab's WebContent process, through WebKit's private
/// `_webProcessIdentifier`; `None` when WebKit no longer answers it.
pub fn web_process_id(tab: TabId) -> Result<Option<i32>, String> {
    with_tab(tab, |view| {
        let selector = objc2::sel!(_webProcessIdentifier);
        let responds: bool = unsafe { msg_send![view, respondsToSelector: selector] };
        if responds {
            // SAFETY: WebKit declares it as `(pid_t)_webProcessIdentifier`.
            let pid: i32 = unsafe { msg_send![view, _webProcessIdentifier] };
            (pid > 0).then_some(pid)
        } else {
            None
        }
    })
}
