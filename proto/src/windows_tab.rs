//! Windows tab views: WebView2 controllers parented to the Tauri window's
//! HWND, made from the UI webview's own environment so they share the
//! profile's browser process.
//!
//! Written on a Mac and not compiled or run there. The Windows run sheet in
//! `mado-docs/research/m0-report.md` says how to build and check it.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use webview2_com::CreateCoreWebView2ControllerCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, ICoreWebView2Environment,
};
use windows::Win32::Foundation::{E_POINTER, HWND, RECT};
use windows::core::HSTRING;

use crate::shell::Rect;

pub type TabId = usize;

static NEXT_TAB: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    // Only the main thread ever touches this list; index = TabId.
    static TABS: RefCell<Vec<Option<ICoreWebView2Controller>>> = const { RefCell::new(Vec::new()) };
}

fn to_physical(rect: Rect, scale: f64) -> RECT {
    // Pixel coordinates fit in i32 on any real screen; truncation is intended.
    #[allow(clippy::cast_possible_truncation)]
    let px = |v: f64| (v * scale).round() as i32;
    RECT {
        left: px(rect.x),
        top: px(rect.y),
        right: px(rect.x + rect.width),
        bottom: px(rect.y + rect.height),
    }
}

/// Creates a tab view over `rect` and loads `url`. The controller is made
/// asynchronously on the main thread; the returned id is valid at once.
pub fn create_tab(window: &tauri::Window, rect: Rect, url: &str) -> Result<TabId, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let ui = window
        .webviews()
        .into_iter()
        .next()
        .ok_or("no UI webview")?;
    let id = NEXT_TAB.fetch_add(1, Ordering::Relaxed);
    let url = url.to_owned();
    ui.with_webview(move |platform| {
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        match attach(platform.environment(), hwnd, to_physical(rect, scale), &url) {
            Ok(controller) => TABS.with_borrow_mut(|tabs| {
                if tabs.len() <= id {
                    tabs.resize(id + 1, None);
                }
                tabs[id] = Some(controller);
            }),
            Err(e) => eprintln!("mado-proto: tab {id}: {e}"),
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(id)
}

fn attach(
    environment: ICoreWebView2Environment,
    hwnd: HWND,
    bounds: RECT,
    url: &str,
) -> Result<ICoreWebView2Controller, String> {
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: a live environment and the window's own HWND.
            unsafe { environment.CreateCoreWebView2Controller(hwnd, &handler) }
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, controller| {
            result?;
            let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )
    .map_err(|e| format!("CreateCoreWebView2Controller: {e:?}"))?;
    let controller = rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "no controller")?
        .map_err(|e| e.to_string())?;
    // SAFETY: COM calls on the controller we just created, on the main thread.
    unsafe {
        controller.SetBounds(bounds).map_err(|e| e.to_string())?;
        controller.SetIsVisible(true).map_err(|e| e.to_string())?;
        controller
            .CoreWebView2()
            .and_then(|w| w.Navigate(&HSTRING::from(url)))
            .map_err(|e| e.to_string())?;
    }
    Ok(controller)
}

fn with_tab<R>(tab: TabId, f: impl FnOnce(&ICoreWebView2Controller) -> R) -> Result<R, String> {
    TABS.with_borrow(|tabs| tabs.get(tab).and_then(Option::as_ref).map(f))
        .ok_or_else(|| format!("tab {tab} is not ready"))
}

pub fn navigate(tab: TabId, url: &str) -> Result<(), String> {
    with_tab(tab, |controller| {
        // SAFETY: COM call on a live controller, on the main thread.
        unsafe {
            controller
                .CoreWebView2()
                .and_then(|w| w.Navigate(&HSTRING::from(url)))
        }
        .map_err(|e| e.to_string())
    })?
}

pub fn set_frame(window: &tauri::Window, tab: TabId, rect: Rect) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    with_tab(tab, |controller| {
        // SAFETY: COM call on a live controller, on the main thread.
        unsafe { controller.SetBounds(to_physical(rect, scale)) }.map_err(|e| e.to_string())
    })?
}

const MANUAL: &str = "the scripted check is macOS only; follow the Windows run sheet";

pub fn clear_navigation(_tab: TabId) {}

pub fn wait_navigation(_tab: TabId, _timeout: Duration) -> Result<Value, String> {
    Err(MANUAL.into())
}

pub fn describe_views(_window: &tauri::Window) -> Result<Value, String> {
    Err(MANUAL.into())
}

pub fn screenshot(
    _app: &tauri::AppHandle,
    _label: &str,
    _path: &std::path::Path,
) -> Result<String, String> {
    Err(MANUAL.into())
}

pub fn environment() -> Value {
    json!({ "os": "windows", "arch": std::env::consts::ARCH })
}
