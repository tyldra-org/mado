//! macOS web extensions through `WKWebExtensionController` (09, M0-T03):
//! load an unpacked Chrome extension, give it the Chrome ID as its unique
//! identifier, grant what it asks for, attach it to tab views, and open its
//! action popup.

use std::cell::RefCell;
use std::path::Path;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionContext, WKWebExtensionContextPermissionStatus,
    WKWebExtensionController, WKWebView, WKWebViewConfiguration,
};
use serde_json::{Value, json};

use crate::macos::{self, TabId};

thread_local! {
    // Main thread only.
    static CONTROLLER: RefCell<Option<Retained<WKWebExtensionController>>> = const { RefCell::new(None) };
    static CONTEXTS: RefCell<Vec<(String, Retained<WKWebExtensionContext>)>> = const { RefCell::new(Vec::new()) };
    static POPUPS: RefCell<Vec<(String, Retained<WKWebView>)>> = const { RefCell::new(Vec::new()) };
}

fn controller() -> Result<Retained<WKWebExtensionController>, String> {
    let mtm = macos::main_thread()?;
    Ok(CONTROLLER.with_borrow_mut(|c| {
        // SAFETY: a plain constructor on the main thread.
        c.get_or_insert_with(|| unsafe { WKWebExtensionController::new(mtm) })
            .clone()
    }))
}

fn errors(list: &objc2_foundation::NSArray<NSError>) -> Vec<String> {
    list.iter()
        .map(|e| format!("{} {} ({})", e.domain(), e.code(), e.localizedDescription()))
        .collect()
}

/// Loads the unpacked extension in `dir` under `chrome_id`. The receiver
/// gets what WebKit reported about it.
pub fn load(dir: &Path, chrome_id: &str) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
    let mtm = macos::main_thread()?;
    let controller = controller()?;
    let (tx, rx) = mpsc::channel();
    let id = chrome_id.to_owned();
    let block = RcBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
        // SAFETY: WebKit passes a valid extension or a valid error.
        let outcome = match (unsafe { Retained::retain(extension) }, unsafe {
            error.as_ref()
        }) {
            (Some(extension), _) => attach(&controller, &extension, &id),
            (None, Some(e)) => Err(format!(
                "{} {} ({})",
                e.domain(),
                e.code(),
                e.localizedDescription()
            )),
            (None, None) => Err("no extension and no error".into()),
        };
        let _ = tx.send(outcome);
    });
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&dir.to_string_lossy()), true);
    // SAFETY: a directory URL and a block that runs on the main thread.
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &block, mtm) };
    Ok(rx)
}

fn attach(
    controller: &WKWebExtensionController,
    extension: &WKWebExtension,
    chrome_id: &str,
) -> Result<Value, String> {
    // SAFETY: WebKit objects used on the main thread, where this block runs.
    unsafe {
        let context = WKWebExtensionContext::contextForExtension(extension);
        context.setUniqueIdentifier(&NSString::from_str(chrome_id));
        let granted = WKWebExtensionContextPermissionStatus::GrantedExplicitly;
        let mut permissions = Vec::new();
        for permission in extension.requestedPermissions().iter() {
            permissions.push(permission.to_string());
            context.setPermissionStatus_forPermission(granted, &permission);
        }
        let mut patterns = Vec::new();
        for pattern in extension.allRequestedMatchPatterns().iter() {
            patterns.push(pattern.string().to_string());
            context.setPermissionStatus_forMatchPattern(granted, &pattern);
        }
        let loaded = controller.loadExtensionContext_error(&context);
        let report = json!({
            "name": extension.displayName().map(|s| s.to_string()),
            "version": extension.version().map(|s| s.to_string()),
            "extensionErrors": errors(&extension.errors()),
            "load": loaded.as_ref().map(|()| "loaded".to_owned()).map_err(|e| e.localizedDescription().to_string()),
            "loaded": context.isLoaded(),
            "uniqueIdentifier": context.uniqueIdentifier().to_string(),
            "baseURL": context.baseURL().absoluteString().map(|s| s.to_string()),
            "contextErrors": errors(&context.errors()),
            "grantedPermissions": permissions,
            "grantedMatchPatterns": patterns,
        });
        CONTEXTS.with_borrow_mut(|c| c.push((chrome_id.to_owned(), context)));
        Ok(report)
    }
}

/// Adds a hidden tab view whose configuration carries the extension
/// controller, so loaded extensions run in it.
pub fn add_tab(window: &tauri::Window) -> Result<TabId, String> {
    let mtm = macos::main_thread()?;
    let content = macos::content_view(window)?;
    let frame = macos::with_tab(0, |v| v.frame())?;
    // SAFETY: plain configuration calls on the main thread.
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    let controller = controller()?;
    // SAFETY: as above.
    unsafe { configuration.setWebExtensionController(Some(&controller)) };
    let id = macos::insert_tab_with(&content, frame, &configuration)?;
    macos::with_tab(id, |v| v.setHidden(true))?;
    Ok(id)
}

/// Opens the extension's default action popup in the window, as Mado shows
/// popups in a popover over the toolbar, and keeps its web view. A popup web
/// view outside any window never renders: Bitwarden's and 1Password's
/// popups stayed blank there.
pub fn open_popup(window: &tauri::Window, chrome_id: &str) -> Result<Value, String> {
    macos::main_thread()?;
    let content = macos::content_view(window)?;
    let context = CONTEXTS
        .with_borrow(|c| {
            c.iter()
                .find(|(id, _)| id == chrome_id)
                .map(|(_, c)| c.clone())
        })
        .ok_or("extension not loaded")?;
    // SAFETY: the default action of a loaded context, on the main thread.
    let action = unsafe { context.actionForTab(None) }.ok_or("no action")?;
    let presents = unsafe { action.presentsPopup() };
    let popup = unsafe { action.popupWebView() };
    let has_popup = popup.is_some();
    if let Some(view) = popup {
        view.setFrame(objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(400.0, 600.0),
        ));
        content.addSubview(&view);
        POPUPS.with_borrow_mut(|p| p.push((chrome_id.to_owned(), view)));
    }
    Ok(json!({
        "presentsPopup": presents,
        "popupWebView": has_popup,
        "badgeText": unsafe { action.badgeText() }.to_string(),
    }))
}

/// Starts the extension's background content (its service worker) and
/// reports whether WebKit could.
pub fn load_background(chrome_id: &str) -> Result<mpsc::Receiver<Result<(), String>>, String> {
    macos::main_thread()?;
    let context = CONTEXTS
        .with_borrow(|c| {
            c.iter()
                .find(|(id, _)| id == chrome_id)
                .map(|(_, c)| c.clone())
        })
        .ok_or("extension not loaded")?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: WebKit passes nil or a valid error.
        let outcome = match unsafe { error.as_ref() } {
            None => Ok(()),
            Some(e) => Err(format!(
                "{} {} ({})",
                e.domain(),
                e.code(),
                e.localizedDescription()
            )),
        };
        let _ = tx.send(outcome);
    });
    // SAFETY: a loaded context and a block that only sends.
    unsafe { context.loadBackgroundContentWithCompletionHandler(&block) };
    Ok(rx)
}

/// Starts script in the extension's popup page.
pub fn eval_popup(
    chrome_id: &str,
    body: &str,
) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
    macos::main_thread()?;
    let view = POPUPS
        .with_borrow(|p| {
            p.iter()
                .find(|(id, _)| id == chrome_id)
                .map(|(_, v)| v.clone())
        })
        .ok_or("no popup")?;
    Ok(macos::start_eval(&view, body))
}
